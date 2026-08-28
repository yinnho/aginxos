/* netd-stub.c — LD_PRELOAD stub for netmgrd's HIDL netd client (M7).
 *
 * netmgrd's NetmgrNetdClientInit() reaches android.system.net.netd@1.1
 * (INetd::getService / INetd::registerForNotifications, HIDL over binder).
 * With no real netd and our fake context manager the reply handling ends in
 * a main-thread SIGSEGV (observed 2026-08-28, right after "Successfully
 * registered for Netd HAL service"). Both symbols are undefined imports of
 * libnetmgr_common.so, so a preload can own them:
 *   - getService()            -> null sp<>  ("service is null" path)
 *   - registerForNotifications-> Return<bool>{value=false, ok=true}
 * Every netd use in netmgr is guarded by mService==NULL, so the data-call
 * path (QMI + netlink + ipacm) runs with netd simply absent — which is the
 * truth on this system anyway.
 * Mangled names copied from `nm -D libnetmgr_common.so`.
 * Build: NDK clang -shared (bionic; preloaded into netmgrd). */
#include <unistd.h>
typedef int unused_t;

/* libbinder's ServiceManagerShim (the C++ binder path, used for e.g.
 * android.system.suspend). Talking to our fake context manager is a race:
 * a reply that one caller parses as BAD_TYPE another parses into a bogus
 * sp<>, and the resulting incStrong->onFirstRef dispatch jumps through a
 * garbage (stack) vtable — observed as the main-thread SIGSEGV at
 * fault addr 0x0. Returning a clean null sp<> skips binder entirely:
 * every netmgr use of these services is null-guarded. sp<IBinder> is a
 * single pointer returned in x0. */
void *_ZN7android18ServiceManagerShim14waitForServiceERKNS_8String16E(void *name)
{
	(void)name;
	write(2, "netd-stub: binder waitForService called\n", 40);
	return 0;
}

void *_ZNK7android18ServiceManagerShim10getServiceERKNS_8String16E(void *self, void *name)
{
	(void)self; (void)name;
	write(2, "netd-stub: binder getService called\n", 36);
	return 0;
}

void *_ZNK7android18ServiceManagerShim12checkServiceERKNS_8String16E(void *self, void *name)
{
	(void)self; (void)name;
	return 0;
}

/* sp<INetd> is a single pointer, returned in x0 — null = not found. */
void *_ZN7android6system3net4netd4V1_15INetd10getServiceERKNSt3__112basic_stringIcNS5_11char_traitsIcEENS5_9allocatorIcEEEEb(void *name, int wait)
{
	(void)name; (void)wait;
	write(2, "netd-stub: getService called\n", 29);
	return 0;
}

/* netmgr_print_cb_tables: imported from libnetmgr_common.so, called from
 * netmgrd main right after the plugin registration phase. If a plugin
 * failed to register (missing/failed .so), the cb tables hold NULLs and
 * the table walker is the main-thread null-deref we see. No-op it — it
 * only prints. */
void netmgr_print_cb_tables(void)
{
	write(2, "netd-stub: print_cb_tables reached\n", 35);
}

/* Return<bool> is 2 bytes ({mSuccess, value}) — under AAPCS64 it comes
 * back in w0: bit0 = mSuccess, bit8 = value. Verified against netmgrd's
 * own call site: `and w9, w0, #0x1; strb w9, [ret]` right after the bl.
 * An sret pointer guess was wrong here and made the stub scribble on the
 * caller's hidl_string — return the int, touch no caller memory. */
int _ZN7android6system3net4netd4V1_15INetd24registerForNotificationsERKNSt3__112basic_stringIcNS5_11char_traitsIcEENS5_9allocatorIcEEEERKNS_2spINS_4hidl7manager4V1_020IServiceNotificationEEE(void *name, void *cb)
{
	(void)name; (void)cb;
	write(2, "netd-stub: registerForNotifications called\n", 43);
	return 1; /* isOk()=true, value=0 (no service yet) */
}
