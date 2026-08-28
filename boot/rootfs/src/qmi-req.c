/* qmi-req: send one or more raw QMI requests to a QRTR node:port over a
 * SINGLE socket (one WDS/UIM client), printing each hex response.
 * usage: qmi-req node port msgid [hextlv|""] [msgid tlv ...]
 * Multiple messages matter when the modem ties state to the client, e.g.
 * WDS Bind Mux Data Port + Start Network must share one client (M7,
 * observed: bind on a fresh client then start on another = INVALID_OPERATION).
 * A msgid of the form "raw:<hex>" sends those exact bytes as the whole
 * frame — used to replay vendor libqmi_cci frames verbatim (they carry a
 * leading txn byte; netmgrd reply analysis, M7).
 * QR_DRAIN=<ms> keeps reading after each response and prints late frames
 * too: PDC (0x2a service) answers in QMI indications, which arrive as
 * separate frames after the bare ack (observed M7, redfin). */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/time.h>
struct sockaddr_qrtr { unsigned short fam; unsigned int node, port; };
static int hexv(int c){ if(c>='0'&&c<='9')return c-'0'; if(c>='a'&&c<='f')return c-'a'+10; if(c>='A'&&c<='F')return c-'A'+10; return -1; }
int main(int argc, char **argv)
{
	int msleep_us;
	const char *e;
	if (argc < 4 || (argc != 4 && (argc - 3) % 2)) { fprintf(stderr,"usage: qmi-req node port msgid [hextlv|\"\"] [msgid tlv ...]\n"); return 1; }
	unsigned node=atoi(argv[1]), port=atoi(argv[2]);
	int fd=socket(42,SOCK_DGRAM,0);
	if (fd<0){perror("socket");return 1;}
	struct sockaddr_qrtr me={42,0,0};
	if (connect(fd,(void*)&me,sizeof me)<0){perror("connect");return 1;}
	struct timeval tv={6,0};
	/* long-running replies (NAS network scan ~30s) need more than the 6s
	 * default; QR_TIMEOUT sets the socket timeout in seconds (M7). */
	if ((e=getenv("QR_TIMEOUT"))) tv.tv_sec=atoi(e);
	setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&tv,sizeof tv);
	struct sockaddr_qrtr dst={42,node,port};
	/* data calls need seconds to establish; QR_SLEEP ms between messages
	 * so bind + start + status polls run on one client (call state is
	 * per-qrtr-client, observed M7: status from a fresh client = err 15) */
	msleep_us = (e=getenv("QR_SLEEP")) ? atoi(e) * 1000 : 0;
	/* QR_DRAIN ms: after the response, read until this short timeout
	 * expires, so indication frames land on stdout as well (PDC, M7). */
	int drain_us = (e=getenv("QR_DRAIN")) ? atoi(e) * 1000 : 0;
	for (int a=3; a<argc; a+=2) {
		if (a > 3 && msleep_us) usleep(msleep_us);
		unsigned char req[512]; unsigned n=0;
		if (!strncmp(argv[a],"raw:",4)) {
			const char *h=argv[a]+4;
			for (unsigned i=0; h[i]&&h[i+1]&&n<sizeof req;i+=2) req[n++]=(hexv(h[i])<<4)|hexv(h[i+1]);
			printf("-> %u:%u raw %u bytes\n",node,port,n);
		} else {
			unsigned msg=strtoul(argv[a],0,0);
			req[0]=0; req[1]=1; req[2]=0; req[3]=msg&0xff; req[4]=msg>>8; n=5;
			unsigned char tlvs[512]; unsigned tn=0;
			const char *h=argv[a+1];
			for (unsigned i=0; h[i]&&h[i+1]&&tn<sizeof tlvs;i+=2) tlvs[tn++]=(hexv(h[i])<<4)|hexv(h[i+1]);
			if (tn) { req[5]=tn&0xff; req[6]=tn>>8; memcpy(req+7,tlvs,tn); n=7+tn; }
			else { req[5]=0; req[6]=0; n=7; }
			printf("-> %u:%u msg=0x%04x tlv=%u bytes\n",node,port,msg,tn);
		}
		if (sendto(fd,req,n,0,(void*)&dst,sizeof dst)<0){perror("sendto");return 1;}
		for (int first=1;;first=0) {
			unsigned char buf[2048];
			ssize_t r=recv(fd,buf,sizeof buf,0);
			if (r<0){ if(first) printf("<- timeout\n"); break; }
			printf("<- %zd bytes:",r);
			for (ssize_t i=0;i<r&&i<1024;i++) printf("%s%02x",(i%16)?" ":"\n   ",buf[i]);
			printf("\n");
			if (!drain_us) break;
			struct timeval dtv={drain_us/1000000,drain_us%1000000};
			setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&dtv,sizeof dtv);
		}
		if (drain_us) setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&tv,sizeof tv);
	}
	return 0;
}
