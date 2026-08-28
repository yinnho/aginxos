/* qmi-req: send one raw QMI request TLV payload to a QRTR node:port, print hex resp. */
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
	if (argc < 4) { fprintf(stderr,"usage: qmi-req node port msgid [hextlv]\n"); return 1; }
	unsigned node=atoi(argv[1]), port=atoi(argv[2]), msg=strtoul(argv[3],0,0);
	unsigned char req[512]; unsigned n=0;
	req[0]=0; req[1]=1; req[2]=0; req[3]=msg&0xff; req[4]=msg>>8; n=5;
	unsigned char tlvs[512]; unsigned tn=0;
	if (argc>4) {
		const char *h=argv[4];
		for (unsigned i=0; h[i]&&h[i+1]&&tn<sizeof tlvs;i+=2) tlvs[tn++]=(hexv(h[i])<<4)|hexv(h[i+1]);
	}
	if (tn) { req[5]=tn&0xff; req[6]=tn>>8; memcpy(req+7,tlvs,tn); n=7+tn; }
	else { req[5]=0; req[6]=0; n=7; }
	int fd=socket(42,SOCK_DGRAM,0);
	if (fd<0){perror("socket");return 1;}
	struct sockaddr_qrtr me={42,0,0};
	if (connect(fd,(void*)&me,sizeof me)<0){perror("connect");return 1;}
	struct timeval tv={4,0}; setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&tv,sizeof tv);
	struct sockaddr_qrtr dst={42,node,port};
	printf("-> %u:%u msg=0x%04x tlv=%u bytes\n",node,port,msg,tn);
	if (sendto(fd,req,n,0,(void*)&dst,sizeof dst)<0){perror("sendto");return 1;}
	unsigned char buf[2048];
	ssize_t r=recv(fd,buf,sizeof buf,0);
	if (r<0){printf("<- timeout\n");return 1;}
	printf("<- %zd bytes:",r);
	for (ssize_t i=0;i<r&&i<128;i++) printf("%s%02x",(i%16)?" ":"\n   ",buf[i]);
	printf("\n");
	return 0;
}
