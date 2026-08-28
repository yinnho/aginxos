/* libnetutils-stub.c — link-only stub for /vendor/bin/netmgrd (M7).
 *
 * redfin's system image ships no libnetutils.so anywhere we mount
 * (checked system_a/system_ext_a/vendor, 2026-08-28), but netmgrd's
 * only import from it is ifc_del_address — interface address removal
 * at teardown. The stub satisfies the dynamic linker; netmgrd reaches
 * its DPM/WDA setup, which is what we need to observe. Real address
 * config on rmnet_ipa0 we do ourselves with ip(8).
 * Build: NDK clang -shared. */
#include <stddef.h>

int ifc_del_address(const char *ifname, const char *addr, int prefixlen)
{
	(void)ifname; (void)addr; (void)prefixlen;
	return 0;
}
