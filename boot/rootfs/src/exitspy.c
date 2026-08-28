/* exitspy.c — LD_PRELOAD: report who calls exit() (M7 debug aid).
 *
 * netmgrd dies with exit(1) after NetmgrNetdClientInit without
 * printing an error anywhere we capture (2026-08-29). This preload
 * intercepts exit()/_exit()/abort(), prints the code and the calling
 * DSO via dladdr, then performs the exit via syscall so behavior is
 * otherwise unchanged.
 * Build: NDK clang -shared. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <unistd.h>

static void report(const char *fn, int code, void *caller)
{
	Dl_info info = { 0 };
	if (dladdr(caller, &info) && info.dli_fname)
		fprintf(stderr, "[X] %s(%d) from %s (%s+0x%lx)\n",
			fn, code, info.dli_fname,
			info.dli_sname ? info.dli_sname : "?",
			caller ? (unsigned long)((char *)caller -
				 (char *)info.dli_fbase) : 0ul);
	else
		fprintf(stderr, "[X] %s(%d) from %p\n", fn, code, caller);
	fflush(stderr);
}

void exit(int code)
{
	report("exit", code, __builtin_return_address(0));
	syscall(SYS_exit_group, code);
	for (;;) pause();
}

void _exit(int code)
{
	report("_exit", code, __builtin_return_address(0));
	syscall(SYS_exit_group, code);
	for (;;) pause();
}

void abort(void)
{
	report("abort", -1, __builtin_return_address(0));
	syscall(SYS_exit_group, 134);
	for (;;) pause();
}
