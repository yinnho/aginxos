/* rmnet-ipa-ctl: the legacy MSM rmnet extended ioctls on the IPA wwan
 * netdev (uapi linux/msm_rmnet.h) — the step netmgrd performs that
 * connects the WAN GSI pipes. Without it every QMAP skb dies in
 * ipa3_wwan_xmit with "pipe 2 not valid" (M7, observed 2026-08-29):
 *   SET_EGRESS_DATA_FORMAT -> handle3_egress_format() -> ipa3_setup_sys_pipe
 *                            (APPS_WAN_PROD, UL pipe)
 *   SET_INGRESS_DATA_FORMAT -> same for APPS_WAN_CONS + default WAN RT tbl
 *   ADD_MUX_CHANNEL(mux 1, "rmnet_data0") -> QMAP hdr + UL flt in IPA
 * Plain QMAP, no checksum offload, no aggregation (matches the SW rmnet
 * module encap and a modem with no WDA aggregation set).
 * usage: rmnet-ipa-ctl <physdev> <vndname> <muxid>
 * Build: zig cc -target aarch64-linux-musl -static. */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <stdint.h>
#include <errno.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <net/if.h>

#define RMNET_IOCTL_EXTENDED 0x000089FD
#define RMNET_IOCTL_SET_EGRESS_DATA_FORMAT 0x0006
#define RMNET_IOCTL_SET_INGRESS_DATA_FORMAT 0x0007
#define RMNET_IOCTL_ADD_MUX_CHANNEL 0x0005
#define RMNET_IOCTL_EGRESS_FORMAT_MAP (1u << 1)
#define RMNET_IOCTL_INGRESS_FORMAT_MAP (1u << 1)
#define RMNET_IOCTL_INGRESS_FORMAT_DEMUXING (1u << 3)

struct rmnet_ioctl_extended_s {
	uint32_t extended_ioctl;
	union {
		uint32_t data;
		int8_t if_name[IFNAMSIZ];
		struct { uint32_t mux_id; int8_t vchannel_name[IFNAMSIZ]; } rmnet_mux_val;
	} u;
};

static int xioctl(int fd, const char *dev, struct rmnet_ioctl_extended_s *e, const char *what)
{
	struct ifreq ifr;
	memset(&ifr, 0, sizeof ifr);
	strncpy(ifr.ifr_name, dev, IFNAMSIZ - 1);
	ifr.ifr_data = (char *)e;
	if (ioctl(fd, RMNET_IOCTL_EXTENDED, &ifr) < 0) {
		fprintf(stderr, "%s: %s: %s\n", dev, what, strerror(errno));
		return -1;
	}
	printf("%s: %s ok\n", dev, what);
	return 0;
}

int main(int argc, char **argv)
{
	if (argc != 4) { fprintf(stderr, "usage: rmnet-ipa-ctl physdev vndname muxid\n"); return 1; }
	unsigned mux = atoi(argv[3]);
	int fd = socket(AF_INET, SOCK_DGRAM, 0);
	if (fd < 0) { perror("socket"); return 1; }
	struct rmnet_ioctl_extended_s e;
	int rc = 0;

	memset(&e, 0, sizeof e);
	e.extended_ioctl = RMNET_IOCTL_SET_EGRESS_DATA_FORMAT;
	e.u.data = RMNET_IOCTL_EGRESS_FORMAT_MAP;
	rc |= xioctl(fd, argv[1], &e, "egress data format");

	memset(&e, 0, sizeof e);
	e.extended_ioctl = RMNET_IOCTL_SET_INGRESS_DATA_FORMAT;
	e.u.data = RMNET_IOCTL_INGRESS_FORMAT_MAP | RMNET_IOCTL_INGRESS_FORMAT_DEMUXING;
	rc |= xioctl(fd, argv[1], &e, "ingress data format");

	memset(&e, 0, sizeof e);
	e.extended_ioctl = RMNET_IOCTL_ADD_MUX_CHANNEL;
	e.u.rmnet_mux_val.mux_id = mux;
	strncpy((char *)e.u.rmnet_mux_val.vchannel_name, argv[2], IFNAMSIZ - 1);
	rc |= xioctl(fd, argv[1], &e, "add mux channel");

	close(fd);
	return rc ? 1 : 0;
}
