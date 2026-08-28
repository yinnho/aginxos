/* coredump-on.c — restore default fatal-signal disposition (LD_PRELOAD).
 *
 * Bionic installs a debuggerd handler for SIGSEGV/SIGABRT that just prints
 * "Fatal signal 11 ..." and exits — no core, so a vendor-daemon crash is a
 * dead end without tombstoned. This constructor puts the signals back to
 * SIG_DFL, so with core_pattern pointed at a file and ulimit -c unlimited
 * the kernel writes an ELF core we can read with lldb on the host.
 * Build: NDK clang -shared (bionic, preloaded into vendor binaries). */
#include <signal.h>

__attribute__((constructor)) static void coredump_on(void)
{
	signal(SIGSEGV, SIG_DFL);
	signal(SIGABRT, SIG_DFL);
	signal(SIGBUS, SIG_DFL);
	signal(SIGFPE, SIG_DFL);
}
