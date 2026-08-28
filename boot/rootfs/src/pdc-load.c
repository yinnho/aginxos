/* pdc-load: push an mcfg carrier config into the modem's PDC store over raw
 * QMI/QRTR — the native equivalent of what rild's mcfg refresh (and the CN
 * carrier Magisk modules) do from the Android side.
 *
 * Protocol per libqmi (qmicli-pdc.c + qmi-service-pdc.json): the config id is
 * the SHA-1 of the file (client-computed), the file goes up in <=1 KiB chunks
 * (msg 0x26), each chunk answered by an indication (flags=04) carrying the
 * remaining size; frame-reset aborts, remaining==0 completes. --select then
 * SET_SELECTEDs (0x23) and ACTIVATEs (0x27) the config — activate applies
 * across the next modem SSR. Frames use the same 7-byte header as qmi-req:
 * flags, txn, msg id big-endian, zero pad, TLV length little-endian.
 *
 * usage: pdc-load <node> <port> <file.mbn> [--select]
 * Rollback: SET_SELECTED 0x23 back to the previous active id (docs/HARDWARE.md
 * keeps redfin's stock active id on record).
 */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>
#include <sys/socket.h>

struct sockaddr_qrtr { unsigned short fam; unsigned int node, port; };

/* ---- compact SHA-1 (FIPS 180-1) ---- */
typedef struct { unsigned h[5]; unsigned long long len; unsigned char buf[64]; unsigned blen; } sha1_t;
static unsigned rol(unsigned v, int s){ return (v<<s)|(v>>(32-s)); }
static void sha1_blk(sha1_t *c, const unsigned char *p){
	unsigned w[80]; unsigned a,b,d,e,f,k,t; int i;
	for(i=0;i<16;i++) w[i]=((unsigned)p[i*4]<<24)|((unsigned)p[i*4+1]<<16)|((unsigned)p[i*4+2]<<8)|p[i*4+3];
	for(;i<80;i++) w[i]=rol(w[i-3]^w[i-8]^w[i-14]^w[i-16],1);
	a=c->h[0];b=c->h[1];d=c->h[2];e=c->h[3];f=c->h[4];
	for(i=0;i<80;i++){
		if(i<20){k=0x5A827999;t=(b&d)|((~b)&e);}
		else if(i<40){k=0x6ED9EBA1;t=b^d^e;}
		else if(i<60){k=0x8F1BBCDC;t=(b&d)|(b&e)|(d&e);}
		else {k=0xCA62C1D6;t=b^d^e;}
		t=rol(a,5)+t+f+k+w[i]; f=e; e=d; d=rol(b,30); b=a; a=t;
	}
	c->h[0]+=a;c->h[1]+=b;c->h[2]+=d;c->h[3]+=e;c->h[4]+=f;
}
static void sha1_init(sha1_t *c){ c->h[0]=0x67452301;c->h[1]=0xEFCDAB89;c->h[2]=0x98BADCFE;c->h[3]=0x10325476;c->h[4]=0xC3D2E1F0; c->len=0;c->blen=0; }
static void sha1_up(sha1_t *c, const unsigned char *p, unsigned n){
	c->len+=n;
	while(n){ unsigned take=64-c->blen; if(take>n) take=n;
		memcpy(c->buf+c->blen,p,take); c->blen+=take; p+=take; n-=take;
		if(c->blen==64){ sha1_blk(c,c->buf); c->blen=0; } }
}
static void sha1_fin(sha1_t *c, unsigned char out[20]){
	unsigned long long bits=c->len*8; int i;
	unsigned char pad=0x80; sha1_up(c,&pad,1);
	unsigned char z=0; while(c->blen!=56) sha1_up(c,&z,1);
	unsigned char lb[8]; for(i=0;i<8;i++) lb[i]=(unsigned char)(bits>>(56-8*i));
	sha1_up(c,lb,8);
	for(i=0;i<5;i++){ out[i*4]=c->h[i]>>24; out[i*4+1]=c->h[i]>>16; out[i*4+2]=c->h[i]>>8; out[i*4+3]=c->h[i]; }
}

/* ---- QMI frame helpers (7-byte header, msg BE, len LE — matches qmi-req) ---- */
static unsigned char frm[2048]; static unsigned frmn; static unsigned char txn;
static void frm_hdr(unsigned msg){
	frm[0]=0; frm[1]=++txn; frm[2]=(msg>>8)&0xff; frm[3]=msg&0xff;
	frm[4]=0; frmn=7; /* len filled by xfer */
}
static void frm_tlv_raw(unsigned char type, const unsigned char *v, unsigned n){
	frm[frmn++]=type; frm[frmn++]=n&0xff; frm[frmn++]=(n>>8)&0xff;
	memcpy(frm+frmn,v,n); frmn+=n;
}

static int fd;
static struct sockaddr_qrtr dst;

/* TLV walk: value starts at +3, advance 3+len (observed on-device). */
static int tlv_of(const unsigned char *b, unsigned n, unsigned char type, unsigned char *v, unsigned cap){
	unsigned i=7;
	while (i+3<=n){
		unsigned ln=b[i+1]|((unsigned)b[i+2]<<8);
		if (b[i]==type){ if (ln>cap) ln=cap; memcpy(v,b+i+3,ln); return ln; }
		i+=3+ln;
	}
	return -1;
}
/* result code: response TLV 0x02 (len 4: res+err) or indication TLV 0x01 (len 2) */
static int res_of(const unsigned char *b, unsigned n){
	unsigned char v[4]; int l=tlv_of(b,n,0x02,v,4);
	if (l==4) return v[0]|(v[1]<<8);
	l=tlv_of(b,n,0x01,v,2);
	if (l==2) return v[0]|(v[1]<<8);
	return -1;
}

/* wait_for: 0 = ack response (flags 02), 1 = indication (flags 04) */
static int xfer(unsigned msg, int want_ind, unsigned char *out, unsigned *outn){
	unsigned char buf[2048];
	frm[5]=(frmn-7)&0xff; frm[6]=((frmn-7)>>8)&0xff;
	if (sendto(fd,frm,frmn,0,(void*)&dst,sizeof dst)<0){ perror("sendto"); return -1; }
	printf("-> msg 0x%04x %u bytes\n",msg,frmn);
	for(;;){
		ssize_t r=recv(fd,buf,sizeof buf,0);
		if (r<0){ perror("recv"); return -1; }
		unsigned fm=((unsigned)buf[2]<<8)|buf[3];
		if (fm!=msg) continue;
		printf("<- %s %zd bytes:",(buf[0]&0x04)?"ind":"rsp",r);
		for (ssize_t i=0;i<r;i++) printf("%s%02x",(i%16)?" ":"\n   ",buf[i]);
		printf("\n");
		if (!want_ind == !(buf[0]&0x04)) { memcpy(out,buf,r); *outn=r; return 0; }
	}
}

int main(int argc, char **argv)
{
	if (argc<4 || argc>5){ fprintf(stderr,"usage: pdc-load node port file.mbn [--select]\n"); return 1; }
	int do_select = (argc==5 && !strcmp(argv[4],"--select"));
	unsigned node=atoi(argv[1]), port=atoi(argv[2]);

	static unsigned char file[512*1024]; unsigned flen=0;
	FILE *f=fopen(argv[3],"rb");
	if (!f){ perror(argv[3]); return 1; }
	flen=fread(file,1,sizeof file,f); fclose(f);
	if (!flen){ fprintf(stderr,"empty file\n"); return 1; }
	unsigned char id[20]; sha1_t sc; sha1_init(&sc); sha1_up(&sc,file,flen); sha1_fin(&sc,id);
	printf("file %s: %u bytes, id ",argv[3],flen);
	for (int i=0;i<20;i++) printf("%02x",id[i]);
	printf("\n");

	fd=socket(42,SOCK_DGRAM,0);
	if (fd<0){ perror("socket"); return 1; }
	struct sockaddr_qrtr me={42,0,0};
	if (connect(fd,(void*)&me,sizeof me)<0){ perror("connect"); return 1; }
	dst.fam=42; dst.node=node; dst.port=port;
	unsigned char ob[2048]; unsigned obn;

	/* register for indications (ack only) */
	frm_hdr(0x20); frm_tlv_raw(0x10,(const unsigned char*)"\x01",1);
	if (xfer(0x20,0,ob,&obn)) return 1;
	if (res_of(ob,obn)!=0){ fprintf(stderr,"register failed\n"); return 1; }

	/* load in chunks; the indication carries the remaining size */
	unsigned off=0;
	unsigned char tok1[4]={0x51,0,0,0}, tok2[4]={0x52,0,0,0}, tok3[4]={0x53,0,0,0};
	while (off<flen){
		unsigned clen=flen-off; if (clen>1024) clen=1024;
		unsigned char tlv[1100]; unsigned tn=0;
		unsigned char le4[4]={1,0,0,0};
		memcpy(tlv+tn,le4,4); tn+=4;                       /* config type: software */
		tlv[tn++]=20; memcpy(tlv+tn,id,20); tn+=20;         /* id */
		le4[0]=flen&0xff;le4[1]=(flen>>8)&0xff;le4[2]=(flen>>16)&0xff;le4[3]=(flen>>24)&0xff;
		memcpy(tlv+tn,le4,4); tn+=4;                       /* total size */
		tlv[tn++]=clen&0xff; tlv[tn++]=(clen>>8)&0xff;     /* chunk */
		memcpy(tlv+tn,file+off,clen); tn+=clen;
		frm_hdr(0x26); frm_tlv_raw(0x01,tlv,tn); frm_tlv_raw(0x10,tok1,4);
		if (xfer(0x26,1,ob,&obn)) return 1;
		int r=res_of(ob,obn);
		if (r){ fprintf(stderr,"chunk at +%u rejected (0x%x)\n",off,r); return 1; }
		unsigned char v[8];
		if (tlv_of(ob,obn,0x13,v,1)==1 && v[0]){ fprintf(stderr,"frame reset — abort\n"); return 1; }
		if (tlv_of(ob,obn,0x12,v,4)!=4){ fprintf(stderr,"no remaining-size TLV — abort\n"); return 1; }
		unsigned rem=v[0]|((unsigned)v[1]<<8)|((unsigned)v[2]<<16)|((unsigned)v[3]<<24);
		printf("   chunk +%u ok, remaining %u\n",off,rem);
		off+=clen;
		if (!rem) break;
	}
	printf("load complete\n");
	if (!do_select) return 0;

	/* set selected: TLV 0x01 = type u32 + u8-prefixed id */
	{
		unsigned char tlv[64]; unsigned tn=0; unsigned char le4[4]={1,0,0,0};
		memcpy(tlv+tn,le4,4); tn+=4; tlv[tn++]=20; memcpy(tlv+tn,id,20); tn+=20;
		frm_hdr(0x23); frm_tlv_raw(0x01,tlv,tn); frm_tlv_raw(0x10,tok2,4);
		if (xfer(0x23,1,ob,&obn)) return 1;
		int r=res_of(ob,obn);
		printf("set-selected result 0x%x\n",r<0?0:r);
		if (r) return 1;
	}
	/* activate: TLV 0x01 = config type u32 */
	{
		frm_hdr(0x27); frm_tlv_raw(0x01,(const unsigned char*)"\x01\x00\x00\x00",4); frm_tlv_raw(0x10,tok3,4);
		if (xfer(0x27,1,ob,&obn)) return 1;
		int r=res_of(ob,obn);
		printf("activate result 0x%x\n",r<0?0:r);
		if (r) return 1;
	}
	printf("done — config should apply across the next modem SSR\n");
	return 0;
}
