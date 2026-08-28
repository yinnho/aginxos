/* logdw-sink.c — stand-in for Android logd's logdw socket (M7).
 *
 * Vendor daemons (netmgrd et al.) log exclusively through liblog to
 * /dev/socket/logdw, a unix datagram socket the real logd owns. On
 * AginxOS nothing serves it: every log call retries connect() forever
 * (the netmgrd spam, observed 2026-08-29) and the log text is lost.
 * This sink binds /dev/socket/logdw, decodes the binary log_msg
 * records, and appends "<prio> tag: message" lines to a file.
 *
 * liblog wire format (per record):
 *   u16 total_len, u16 log_id, u32 tid, i64 sec, i64 nsec, u8 prio,
 *   then "tag\0message\0".
 * Build: NDK clang (bionic) or musl-static; run: logdw-sink <outfile>
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

int main(int argc, char **argv)
{
	const char *out = argc > 1 ? argv[1] : "/tmp/logd.txt";
	struct sockaddr_un sa = { .sun_family = AF_UNIX };
	int fd, on = 1;
	unsigned char buf[4096];

	strcpy(sa.sun_path, "/dev/socket/logdw");
	fd = socket(AF_UNIX, SOCK_DGRAM, 0);
	if (fd < 0) {
		perror("socket");
		return 1;
	}
	setsockopt(fd, SOL_SOCKET, SO_PASSCRED, &on, sizeof(on));
	unlink(sa.sun_path);
	if (bind(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		perror("bind /dev/socket/logdw");
		return 1;
	}
	chmod(sa.sun_path, 0666);

	for (;;) {
		ssize_t n = recv(fd, buf, sizeof(buf), 0);
		unsigned len, hdrlen;
		char *tag, *msg;
		FILE *f;
		if (n <= 4)
			continue;
		len = (unsigned)buf[0] | ((unsigned)buf[1] << 8);
		if (len > (unsigned)n)
			len = (unsigned)n;
		hdrlen = 2 + 2 + 4 + 16 + 1; /* len,id,tid,timespec,prio */
		if (len <= hdrlen + 2)
			continue;
		tag = (char *)buf + hdrlen;
		msg = tag + strlen(tag) + 1;
		if (msg >= (char *)buf + len)
			continue;
		f = fopen(out, "a");
		if (!f)
			continue;
		fprintf(f, "%c %s: %.*s\n", buf[hdrlen - 1] >= ' ' ? 'L' : ' ',
			tag, (int)((char *)buf + len - msg), msg);
		fclose(f);
	}
}
