/* sock-trace.c — LD_PRELOAD socket/connect/bind/sendto tracer (M7).
 *
 * trace_open.so covers file opens; the vendor data stack's next unknowns
 * are socket-shaped: which transport libqmi_cci picks (AF_QIPCRTR vs
 * /dev/socket/qmux_*), and whether dsi-call ever reaches netmgrd's
 * /dev/socket/netmgr/* sockets. This preload prints socket(AF_*),
 * connect(), bind(), sendto/recvfrom/sendmsg/recvmsg results to stderr
 * (msg variants added for netmgrd — libqmi_cci on QRTR uses sendmsg).
 * Build: NDK clang -shared. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int (*real_socket)(int, int, int);
static int (*real_connect)(int, const struct sockaddr *, socklen_t);
static int (*real_bind)(int, const struct sockaddr *, socklen_t);
static ssize_t (*real_sendto)(int, const void *, size_t, int,
			      const struct sockaddr *, socklen_t);
static ssize_t (*real_recvfrom)(int, void *, size_t, int,
				struct sockaddr *, socklen_t *);
static ssize_t (*real_sendmsg)(int, const struct msghdr *, int);
static ssize_t (*real_recvmsg)(int, struct msghdr *, int);
static int (*real_bind)(int, const struct sockaddr *, socklen_t);
static ssize_t (*real_sendto)(int, const void *, size_t, int,
			      const struct sockaddr *, socklen_t);
static ssize_t (*real_recvfrom)(int, void *, size_t, int,
				struct sockaddr *, socklen_t *);

/* qrtr wire: ctrl pkt {u32 cmd; u32 service; u32 instance; u32 node; u32 port}
 * — a 20B QIPCRTR sendto is a NEW_LOOKUP/DEL_LOOKUP; the first payload
 * words name the service netmgrd is hunting. QMI-over-qrtr data packets
 * start 01 xx xx (QMI SDU). */
static void hex32(const void *buf, size_t n)
{
	const unsigned char *b = buf;
	size_t i, m = n > 128 ? 128 : n;
	fprintf(stderr, " [");
	for (i = 0; i < m; i++)
		fprintf(stderr, "%02x", b[i]);
	fprintf(stderr, "]");
}

static void init_syms(void)
{
	if (!real_socket)
		real_socket = dlsym(RTLD_NEXT, "socket");
	if (!real_connect)
		real_connect = dlsym(RTLD_NEXT, "connect");
	if (!real_bind)
		real_bind = dlsym(RTLD_NEXT, "bind");
	if (!real_sendto)
		real_sendto = dlsym(RTLD_NEXT, "sendto");
	if (!real_recvfrom)
		real_recvfrom = dlsym(RTLD_NEXT, "recvfrom");
	if (!real_sendmsg)
		real_sendmsg = dlsym(RTLD_NEXT, "sendmsg");
	if (!real_recvmsg)
		real_recvmsg = dlsym(RTLD_NEXT, "recvmsg");
}

static const char *fam(int f)
{
	switch (f) {
	case 1: return "UNIX";
	case 2: return "INET";
	case 10: return "INET6";
	case 16: return "NETLINK";
	case 42: return "QIPCRTR";
	default: return "?";
	}
}

int socket(int domain, int type, int protocol)
{
	int r;
	init_syms();
	r = real_socket(domain, type, protocol);
	fprintf(stderr, "[S] socket(%s,%d,%d) = %d errno %d\n",
		fam(domain), type, protocol, r, errno);
	return r;
}

int connect(int fd, const struct sockaddr *sa, socklen_t len)
{
	int r;
	init_syms();
	r = real_connect(fd, sa, len);
	if (sa->sa_family == AF_UNIX) {
		const struct sockaddr_un *u = (const void *)sa;
		char nm[sizeof(u->sun_path) + 1];
		memcpy(nm, u->sun_path, sizeof(u->sun_path));
		nm[sizeof(u->sun_path)] = 0;
		if (!nm[0] && len > 3) {
			/* abstract: name is sun_path+1 up to addr len */
			size_t an = (size_t)len - 3;
			unsigned k;
			if (an > sizeof(nm) - 1)
				an = sizeof(nm) - 1;
			memcpy(nm, u->sun_path + 1, an);
			nm[an] = 0;
			fprintf(stderr, "[S] connect(%d, abstract \"%s\" len %u raw",
				fd, nm, len);
			for (k = 0; k < 16 && k < len - 2; k++)
				fprintf(stderr, " %02x",
					(unsigned char)u->sun_path[k]);
			fprintf(stderr, ") = %d errno %d\n", fd, r, errno);
			(void)fd;
		} else {
			fprintf(stderr, "[S] connect(%d, unix %s) = %d errno %d\n",
				fd, nm[0] ? nm : "(empty)", r, errno);
		}
	} else {
		fprintf(stderr, "[S] connect(%d, fam %s) = %d errno %d\n",
			fd, fam(sa->sa_family), r, errno);
	}
	return r;
}

int bind(int fd, const struct sockaddr *sa, socklen_t len)
{
	int r;
	init_syms();
	r = real_bind(fd, sa, len);
	if (sa->sa_family == AF_UNIX) {
		const struct sockaddr_un *u = (const void *)sa;
		char nm[sizeof(u->sun_path) + 1];
		memcpy(nm, u->sun_path, sizeof(u->sun_path));
		nm[sizeof(u->sun_path)] = 0;
		fprintf(stderr, "[S] bind(%d, unix %s) = %d errno %d\n",
			fd, nm[0] ? nm : "(abstract)", r, errno);
	} else {
		fprintf(stderr, "[S] bind(%d, fam %s) = %d errno %d\n",
			fd, fam(sa->sa_family), r, errno);
	}
	return r;
}

ssize_t sendto(int fd, const void *buf, size_t n, int flags,
	       const struct sockaddr *sa, socklen_t len)
{
	ssize_t r;
	init_syms();
	r = real_sendto(fd, buf, n, flags, sa, len);
	if (sa) {
		fprintf(stderr, "[S] sendto(%d, %zuB, fam %s) = %zd errno %d",
			fd, n, fam(sa->sa_family), r, errno);
		if (sa->sa_family == 42 /* AF_QIPCRTR */ && buf && n >= 4)
			hex32(buf, n);
		fputc('\n', stderr);
	} else {
		fprintf(stderr, "[S] send(%d, %zuB) = %zd errno %d",
			fd, n, r, errno);
		if (buf && n >= 4 && fd != 2)
			hex32(buf, n);
		fputc('\n', stderr);
	}
	return r;
}

/* replies arriving on qrtr/netlink sockets: NEW_SERVER advertisements and
 * QMI responses. sa==NULL means a plain recv(). */
ssize_t recvfrom(int fd, void *buf, size_t n, int flags,
		 struct sockaddr *sa, socklen_t *lenp)
{
	ssize_t r;
	init_syms();
	r = real_recvfrom(fd, buf, n, flags, sa, lenp);
	if (r > 0) {
		fprintf(stderr, "[S] recvfrom(%d) = %zd", fd, r);
		if (sa && lenp && *lenp >= 8 && sa->sa_family == 42) {
			const unsigned *q = (const void *)sa;
			fprintf(stderr, " qrtr node %u port %u", q[1], q[2]);
		}
		if (r >= 4)
			hex32(buf, (size_t)r);
		fputc('\n', stderr);
	} else if (r < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
		fprintf(stderr, "[S] recvfrom(%d) = %zd errno %d\n", fd, r, errno);
	}
	return r;
}

/* msg variants — netmgrd's libqmi_cci QRTR transport path. */
ssize_t sendmsg(int fd, const struct msghdr *msg, int flags)
{
	ssize_t r;
	init_syms();
	r = real_sendmsg(fd, msg, flags);
	if (msg) {
		const struct sockaddr *sa = msg->msg_name;
		fprintf(stderr, "[S] sendmsg(%d, %u iov, fam %s) = %zd errno %d",
			fd, msg->msg_iovlen, sa ? fam(sa->sa_family) : "anon",
			r, errno);
		if (sa && sa->sa_family == 42 /* AF_QIPCRTR */) {
			const unsigned *q = (const void *)sa;
			fprintf(stderr, " qrtr node %u port %u", q[1], q[2]);
		}
		if (msg->msg_iovlen == 1 && msg->msg_iov && msg->msg_iov[0].iov_base)
			hex32(msg->msg_iov[0].iov_base,
			      (size_t)msg->msg_iov[0].iov_len);
		fputc('\n', stderr);
	}
	return r;
}

ssize_t recvmsg(int fd, struct msghdr *msg, int flags)
{
	ssize_t r;
	init_syms();
	r = real_recvmsg(fd, msg, flags);
	if (r > 0) {
		const struct sockaddr *sa = msg ? msg->msg_name : NULL;
		fprintf(stderr, "[S] recvmsg(%d) = %zd", fd, r);
		if (sa && msg->msg_namelen >= 16 && sa->sa_family == 42) {
			const unsigned *q = (const void *)sa;
			fprintf(stderr, " qrtr node %u port %u", q[1], q[2]);
		}
		if (msg && msg->msg_iovlen == 1 && msg->msg_iov &&
		    msg->msg_iov[0].iov_base)
			hex32(msg->msg_iov[0].iov_base, (size_t)r);
		fputc('\n', stderr);
	} else if (r < 0 && errno != EAGAIN && errno != EWOULDBLOCK) {
		fprintf(stderr, "[S] recvmsg(%d) = %zd errno %d\n", fd, r, errno);
	}
	return r;
}
