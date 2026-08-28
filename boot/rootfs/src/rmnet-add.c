/* rmnet-add: create an rmnet virtual netdev over the IPA wwan device via
 * rtnetlink, e.g. `rmnet-add rmnet_ipa0 rmnet_data0 1`.
 *
 * M7: ipa3_wwan_xmit() silently drops every skb whose protocol is not
 * ETH_P_MAP (SW filtering, tx_dropped, no log line) and the RX callback
 * hands DL packets up as ETH_P_MAP — so raw-IP traffic never works on
 * rmnet_ipa0 directly. The rmnet kernel module does QMAP encap/decap on
 * a child netdev (mux id must match the WDS BIND_MUX_DATA_PORT id).
 * busybox `ip link add` has no `type rmnet`, so this issues the
 * RTM_NEWLINK by hand (observed need, 2026-08-29).
 * Build: zig cc -target aarch64-linux-musl -static. */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <linux/netlink.h>

/* not in musl's if_link.h */
#define IFLA_IFNAME_    3
#define IFLA_LINK_      5
#define IFLA_LINKINFO_  18
#define IFLA_INFO_KIND_ 1
#define IFLA_INFO_DATA_ 2
#define IFLA_RMNET_MUX_ID_ 1

struct ifinfomsg_ { unsigned char family, __pad1; unsigned short __pad2; int ifindex, flags, change; };

static int nlsk(void)
{
	int fd = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
	if (fd < 0) { perror("socket"); return -1; }
	struct sockaddr_nl sa = { .nl_family = AF_NETLINK };
	if (bind(fd, (void *)&sa, sizeof sa) < 0) { perror("bind"); return -1; }
	return fd;
}

static char *attr(char *p, unsigned short type, const void *val, unsigned short len)
{
	struct nlattr *a = (struct nlattr *)p;
	a->nla_len = NLMSG_ALIGN(sizeof *a) + len;
	a->nla_type = type;
	memcpy(p + NLMSG_ALIGN(sizeof *a), val, len);
	return p + NLMSG_ALIGN(a->nla_len);
}

int main(int argc, char **argv)
{
	if (argc != 4) { fprintf(stderr, "usage: rmnet-add realdev newname muxid\n"); return 1; }
	char path[128];
	snprintf(path, sizeof path, "/sys/class/net/%s/ifindex", argv[1]);
	FILE *f = fopen(path, "r");
	if (!f) { perror(argv[1]); return 1; }
	int link;
	if (fscanf(f, "%d", &link) != 1) { fprintf(stderr, "bad ifindex\n"); return 1; }
	fclose(f);
	unsigned mux = atoi(argv[3]);

	/* nested IFLA_INFO_DATA { IFLA_RMNET_MUX_ID } inside
	 * IFLA_LINKINFO { IFLA_INFO_KIND "rmnet", INFO_DATA } */
	static char data[64], info[128], buf[512];
	/* IFLA_RMNET_MUX_ID is read cpu-order (nla_get_u16, like iproute2's
	 * addattr16) — htons() here made mux 1 read as 256 → netlink -ERANGE */
	char *e = attr(data, IFLA_RMNET_MUX_ID_, &(unsigned short){ mux }, 2);
	char *kind = "rmnet";
	char *i = attr(info, IFLA_INFO_KIND_, kind, strlen(kind) + 1);
	i = attr(i, IFLA_INFO_DATA_, data, (unsigned short)(e - data));

	struct ifinfomsg_ ifi = { 0 };
	struct nlmsghdr *nh = (struct nlmsghdr *)buf;
	memset(buf, 0, sizeof buf);
	*nh = (struct nlmsghdr){ .nlmsg_len = NLMSG_ALIGN(sizeof *nh) + sizeof ifi,
		.nlmsg_type = 16 /* RTM_NEWLINK */,
		.nlmsg_flags = NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
		.nlmsg_seq = 1 };
	memcpy(buf + NLMSG_ALIGN(sizeof *nh), &ifi, sizeof ifi);
	char *p = buf + NLMSG_ALIGN(sizeof *nh) + sizeof ifi;
	p = attr(p, IFLA_IFNAME_, argv[2], strlen(argv[2]) + 1);
	p = attr(p, IFLA_LINK_, &link, 4); /* cpu-order like iproute2's addattr32 */
	p = attr(p, IFLA_LINKINFO_, info, (unsigned short)(i - info));
	nh->nlmsg_len = p - buf;

	int fd = nlsk();
	if (fd < 0) return 1;
	struct sockaddr_nl kern = { .nl_family = AF_NETLINK };
	if (sendto(fd, buf, nh->nlmsg_len, 0, (void *)&kern, sizeof kern) < 0) { perror("sendto"); return 1; }
	char rbuf[1024];
	ssize_t r = recv(fd, rbuf, sizeof rbuf, 0);
	if (r < 0) { perror("recv"); return 1; }
	struct nlmsghdr *rn = (struct nlmsghdr *)rbuf;
	if (rn->nlmsg_type == NLMSG_ERROR) {
		struct nlmsgerr *err = NLMSG_DATA(rn);
		if (err->error == 0) { printf("created %s mux %u over %s\n", argv[2], mux, argv[1]); return 0; }
		fprintf(stderr, "netlink error %d (%s)\n", err->error, strerror(-err->error));
		return 1;
	}
	fprintf(stderr, "unexpected netlink reply type %d\n", rn->nlmsg_type);
	return 1;
}
