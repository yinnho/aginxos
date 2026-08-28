/* diag-stub.c — LD_PRELOAD vendor shims for netmgrd (M7).
 *
 * Diag_LSM_Init: libdiag connects to the diag-router abstract socket
 * (nothing serves it on AginxOS — sun_path all-zero, connect(2) hits
 * ECONNREFUSED, observed 2026-08-29). configdb_init() hard-fails when
 * Diag_LSM_Init fails, and netmgrd aborts when configdb can't load
 * /vendor/etc/data/netmgr_config.xml — so without this shim netmgrd
 * dies before ever reaching its DPM/WDA setup. DIAG is the QXDM
 * logging channel; lying here only costs log forwarding, which we
 * have no consumer for anyway.
 *
 * property_set: netmgr_main_uplink_priority_init() returns
 * NETMGR_FAILURE when property_set(vendor...uplink...) fails
 * (lito config has uplink_priority=1), which makes main() return 1
 * and netmgrd exit silently right before creating its unix listener
 * (observed 2026-08-29 — exit(1) with no error line anywhere). We
 * have no property service, so accept-and-drop every set.
 * Build: NDK clang -shared. */
#include <stdio.h>

int Diag_LSM_Init(void *UNUSED)
{
	(void)UNUSED;
	return 1; /* TRUE */
}

void Diag_LSM_DeInit(void)
{
}

int property_set(const char *key, const char *value)
{
	static FILE *f;
	if (!f) f = fopen("/tmp/propset.log", "a");
	if (f) { fprintf(f, "set %s = %s\n", key ? key : "?", value ? value : "?"); fflush(f); }
	return 0; /* success */
}
