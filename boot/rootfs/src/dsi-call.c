/* dsi-call: start a cellular data call through the vendor DSI library
 * (libdsi_netctrl.so) — the same client API netmgrd itself uses.
 *
 * Manual QMI (qmi-req) proved M6: SIM present (China Telecom), LTE
 * REGISTERED_HOME, PS attached. But a raw WDS START_NETWORK keeps answering
 * INVALID_OPERATION while IPA/rmnet sits unconfigured (observed 2026-08-28),
 * because the modem-side data path expects the vendor bring-up dance
 * (embedded call type, endpoint/mux registration, ipacm coordination) that
 * DSI performs. This tool links the vendor library instead of re-speaking
 * the protocol: dsi_init -> get handle -> set APN + IP family + embedded
 * call type -> start, then reports the netdev name and the assigned
 * addresses and stays alive so the call persists (kill to stop).
 *
 * Build (NDK, bionic — runs against /vendor_a/lib64):
 *   clang --target=aarch64-linux-android24 dsi-call.c libdsi_netctrl.so \
 *     -Wl,--allow-shlib-undefined -o dsi-call
 * usage: dsi-call <apn> [4|6|46] [connect_wait_secs]
 */
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include "dsi_netctrl.h"

static volatile int conn_state; /* 0 = pending, 1 = conn, -1 = no net */

static void ev_cb(dsi_hndl_t hndl, void *user, dsi_net_evt_t evt,
                  dsi_evt_payload_t *payload)
{
	const char *n = "other";
	switch (evt) {
	case DSI_EVT_NET_IS_CONN:    n = "NET_IS_CONN";    conn_state = 1;  break;
	case DSI_EVT_NET_NO_NET:     n = "NET_NO_NET";     conn_state = -1; break;
	case DSI_EVT_WDS_CONNECTED:  n = "WDS_CONNECTED";  break;
	case DSI_EVT_NET_NEWADDR:    n = "NET_NEWADDR";    break;
	case DSI_EVT_NET_PARTIAL_CONN: n = "NET_PARTIAL_CONN"; break;
	case DSI_EVT_NET_RECONFIGURED: n = "NET_RECONFIGURED"; break;
	case DSI_EVT_NET_NEWMTU:     n = "NET_NEWMTU";     break;
	default: break;
	}
	printf("dsi event: %d %s\n", (int)evt, n);
	fflush(stdout);
	(void)hndl; (void)user; (void)payload;
}

static void print_addrs(dsi_hndl_t h)
{
	unsigned int cnt = dsi_get_ip_addr_count(h);
	printf("ip addr count: %u\n", cnt);
	if (!cnt) return;
	dsi_addr_info_t info[4];
	int n = dsi_get_ip_addr(h, info, cnt < 4 ? cnt : 4);
	if (n <= 0) { printf("dsi_get_ip_addr: %d\n", n); return; }
	for (int i = 0; i < n; i++) {
		char ab[INET6_ADDRSTRLEN] = "?";
		struct sockaddr *sa = &info[i].iface_addr_s.addr;
		if (sa->sa_family == AF_INET)
			inet_ntop(AF_INET, &((struct sockaddr_in *)sa)->sin_addr, ab, sizeof ab);
		else if (sa->sa_family == AF_INET6)
			inet_ntop(AF_INET6, &((struct sockaddr_in6 *)sa)->sin6_addr, ab, sizeof ab);
		printf("addr[%d]: %s\n", i, ab);
	}
}

int main(int argc, char **argv)
{
	if (argc < 2) {
		fprintf(stderr, "usage: dsi-call <apn> [4|6|46] [connect_wait_secs]\n");
		return 1;
	}
	const char *apn = argv[1];
	int ipver = DSI_IP_VERSION_4;
	if (argc > 2 && !strcmp(argv[2], "6"))  ipver = DSI_IP_VERSION_6;
	if (argc > 2 && !strcmp(argv[2], "46")) ipver = DSI_IP_VERSION_4_6;
	int wait = argc > 3 ? atoi(argv[3]) : 120;

	if (dsi_init(DSI_MODE_GENERAL) != DSI_SUCCESS) {
		fprintf(stderr, "dsi_init failed\n");
		return 1;
	}
	printf("dsi_init ok\n");

	dsi_hndl_t h = dsi_get_data_srvc_hndl(ev_cb, NULL);
	if (!h) { fprintf(stderr, "dsi_get_data_srvc_hndl failed\n"); return 1; }
	printf("hndl %p\n", h);

	dsi_call_param_value_t v;
	memset(&v, 0, sizeof v);
	v.buf_val = (char *)apn; v.num_val = (int)strlen(apn);
	if (dsi_set_data_call_param(h, DSI_CALL_INFO_APN_NAME, &v)) {
		fprintf(stderr, "set APN failed\n"); return 1;
	}
	v.buf_val = NULL; v.num_val = ipver;
	if (dsi_set_data_call_param(h, DSI_CALL_INFO_IP_VERSION, &v))
		printf("note: IP_VERSION param refused (defaulting)\n");
	v.buf_val = NULL; v.num_val = DSI_CALL_TYPE_EMBEDDED;
	if (dsi_set_data_call_param(h, DSI_CALL_INFO_CALL_TYPE, &v))
		printf("note: CALL_TYPE param refused (defaulting)\n");
	printf("starting call: apn=%s ipver=%d type=embedded\n", apn, ipver);

	if (dsi_start_data_call(h)) {
		fprintf(stderr, "dsi_start_data_call failed\n");
		return 1;
	}
	for (int t = 0; t < wait && !conn_state; t++) sleep(1);
	if (conn_state != 1) {
		fprintf(stderr, "no DSI_EVT_NET_IS_CONN within %ds (state %d)\n", wait, conn_state);
		print_addrs(h);
		return 2;
	}

	char dev[DSI_CALL_INFO_DEVICE_NAME_MAX_LEN + 1];
	if (dsi_get_device_name(h, dev, sizeof dev) == DSI_SUCCESS)
		printf("netdev: %s\n", dev);
	print_addrs(h);
	printf("connected — staying alive (kill to end the call)\n");
	fflush(stdout);
	for (;;) sleep(60);
	return 0;
}
