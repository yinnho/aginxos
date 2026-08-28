/* crash-tracer.c — ptrace-based crash catcher for vendor binaries (M7).
 *
 * The kernel has CONFIG_COREDUMP off and there is no tombstoned, so a
 * netmgrd SIGSEGV is a one-line "Fatal signal 11, fault addr 0x0" with no
 * PC and no backtrace. This tool forks, PTRACE_TRACEMEs, execs the target,
 * follows clones, and on the first fatal signal prints registers (pc/lr/fp),
 * the fault address from siginfo, and a frame-pointer backtrace, then kills
 * the tracee. LD_* and friends come from the caller's environment.
 *
 * Usage: crash-tracer <prog> [args...]
 * Build: NDK clang (bionic).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>
#include <signal.h>
#include <sys/ptrace.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <sys/uio.h>

struct user_pt { unsigned long x[31]; unsigned long pc, pstate; };

static struct user_pt get_regs(pid_t pid)
{
	struct user_pt pt = {0};
	struct iovec iov = { &pt, sizeof(pt) };
	ptrace(PTRACE_GETREGSET, pid, (void *)1 /*NT_PRSTATUS*/, &iov);
	return pt;
}

static unsigned long peek(pid_t pid, unsigned long addr)
{
	errno = 0;
	unsigned long v = (unsigned long)ptrace(PTRACE_PEEKDATA, pid, (void *)addr, 0);
	return v;
}

static void poke(pid_t pid, unsigned long addr, unsigned long val)
{
	ptrace(PTRACE_POKETEXT, pid, (void *)addr, (void *)val);
}

/* find load base of `name` from /proc/pid/maps (first mapping, offset 0) */
static unsigned long find_base(pid_t pid, const char *name)
{
	char path[48], line[1024];
	FILE *f;
	unsigned long lo = 0, off;
	snprintf(path, sizeof(path), "/proc/%d/maps", pid);
	f = fopen(path, "r");
	if (!f)
		return 0;
	while (fgets(line, sizeof(line), f)) {
		if (strstr(line, name) && strstr(line, "r--p")) {
			sscanf(line, "%lx-%*x %*s %lx", &lo, &off);
			if (off == 0)
				break;
			lo = 0;
		}
	}
	fclose(f);
	return lo;
}

static void dump_all_regs(pid_t w, struct user_pt *pt, const char *tag)
{
	int i;
	fprintf(stderr, "crash-tracer: [%s] pc=0x%lx sp~fp=0x%lx\n", tag, pt->pc, pt->x[29]);
	for (i = 0; i < 31; i++)
		fprintf(stderr, "  x%-2d=0x%lx%s", i, pt->x[i], (i % 4 == 3) ? "\n" : "  ");
	fprintf(stderr, "\n");
}

int main(int argc, char **argv)
{
	pid_t pid;
	int status;
	unsigned long break_vaddr = 0, base = 0, orig_word = 0, break_addr = 0;
	int step_trace = 0;
	const char *e, *patches = NULL;

	if (argc < 2) {
		fprintf(stderr, "usage: %s prog [args]  (env: CT_BREAK=<vaddr> CT_TRACE=1)\n", argv[0]);
		return 2;
	}
	if ((e = getenv("CT_BREAK")))
		break_vaddr = strtoul(e, 0, 0);
	if ((e = getenv("CT_TRACE")))
		step_trace = atoi(e);
	patches = getenv("CT_PATCHES");

	pid = fork();
	if (pid == 0) {
		ptrace(PTRACE_TRACEME, 0, 0, 0);
		signal(SIGSEGV, SIG_DFL);
		signal(SIGABRT, SIG_DFL);
		signal(SIGBUS, SIG_DFL);
		execv(argv[1], &argv[1]);
		perror("execv");
		_exit(127);
	}

	/* initial exec stop */
	if (waitpid(pid, &status, 0) < 0) {
		perror("waitpid");
		return 1;
	}
	ptrace(PTRACE_SETOPTIONS, pid, 0,
	       PTRACE_O_TRACECLONE | PTRACE_O_TRACEEXEC | PTRACE_O_EXITKILL);

	/* after exec we are at the entry stop: apply code patches, then
	 * optionally breakpoint; if only patches were requested, detach and
	 * let the tracee run free (patches persist in its memory) */
	if (patches) {
		char *spec = strdup(patches), *tok, *save = NULL;
		base = find_base(pid, argv[1]);
		for (tok = spec; (tok = strtok_r(tok, ",; ", &save)); tok = NULL) {
			unsigned long va = strtoul(tok, &tok, 0), nw;
			unsigned long addr, old;
			if (*tok != '=' && *tok != ':') continue;
			nw = strtoul(tok + 1, 0, 0);
			addr = base + va;
			old = peek(pid, addr);
			poke(pid, addr, (old & ~0xffffffffUL) | (nw & 0xffffffffUL));
			fprintf(stderr, "crash-tracer: patched 0x%lx: 0x%08x -> 0x%08lx\n",
				addr, (unsigned)(old & 0xffffffff), nw & 0xffffffff);
		}
		free(spec);
	}
	if (break_vaddr) {
		base = find_base(pid, argv[1]);
		break_addr = base + break_vaddr;
		orig_word = peek(pid, break_addr);
		poke(pid, break_addr, (orig_word & ~0xffffffffUL) | 0xd4200000UL);
		fprintf(stderr, "crash-tracer: brk at 0x%lx (base 0x%lx + 0x%lx), orig 0x%lx\n",
			break_addr, base, break_vaddr, orig_word);
	}
	if (!break_vaddr && patches) {
		/* patch-only mode: let it run free */
		ptrace(PTRACE_DETACH, pid, 0, 0);
		fprintf(stderr, "crash-tracer: detached, pid %d running patched\n", pid);
		return 0;
	}
	/* the exec stop is consumed above; restart before waiting again */
	ptrace(PTRACE_CONT, pid, 0, 0);

	for (;;) {
		pid_t w = waitpid(-1, &status, __WALL);
		int sig, deliver = 0;

		if (w < 0) {
			perror("waitpid");
			return 1;
		}
		if (WIFEXITED(status) || WIFSIGNALED(status)) {
			fprintf(stderr, "crash-tracer: tid %d gone (rc=%d sig=%d)\n", w,
				WIFEXITED(status) ? WEXITSTATUS(status) : WTERMSIG(status),
				WIFSIGNALED(status) ? WTERMSIG(status) : 0);
			if (w == pid)
				return 0;
			continue;
		}
		sig = WSTOPSIG(status);

		/* breakpoint hit? */
		if (sig == SIGTRAP && w == pid) {
			struct user_pt pt = get_regs(w);
			if (break_addr && pt.pc == break_addr + 4) {
				fprintf(stderr, "crash-tracer: *** breakpoint hit ***\n");
				poke(w, break_addr, orig_word);
				pt.pc = break_addr; /* rewind over the brk */
				{
					struct iovec iov = { &pt, sizeof(pt) };
					ptrace(PTRACE_SETREGSET, w, (void *)1, &iov);
				}
				dump_all_regs(w, &pt, "break");
				/* dump vtable chain of x0 and [x19] */
				fprintf(stderr, "  [x0]=0x%lx [x0+8]=0x%lx [x0+0x18]=0x%lx [x19]=0x%lx\n",
					peek(w, pt.x[0]), peek(w, pt.x[0] + 8),
					peek(w, pt.x[0] + 0x18), peek(w, pt.x[19]));
				if (step_trace) {
					int n;
					unsigned long last_hi = 0;
					for (n = 0; n < 20000; n++) {
						struct user_pt s;
						if (ptrace(PTRACE_SINGLESTEP, w, 0, 0) < 0)
							break;
						if (waitpid(w, &status, 0) < 0)
							break;
						if (WIFEXITED(status) || WIFSIGNALED(status)) {
							fprintf(stderr, "crash-tracer: died stepping rc/sig=%d\n",
								WIFSIGNALED(status) ? WTERMSIG(status) : 0);
							return 3;
						}
						if (WSTOPSIG(status) == SIGSEGV || WSTOPSIG(status) == SIGBUS ||
						    WSTOPSIG(status) == SIGILL) {
							siginfo_t si;
							struct user_pt f;
							memset(&si, 0, sizeof(si));
							ptrace(PTRACE_GETSIGINFO, w, 0, &si);
							f = get_regs(w);
							fprintf(stderr, "crash-tracer: fault at step %d: sig %d code %d addr %p\n",
								n, WSTOPSIG(status), si.si_code, si.si_addr);
							dump_all_regs(w, &f, "step-fault");
							/* 8 words of code at pc */
							{
								int k;
								for (k = -2; k < 6; k++)
									fprintf(stderr, "  mem[pc%+d]=0x%016lx\n", k * 4, peek(w, f.pc + k * 4));
							}
							kill(pid, SIGKILL);
							while (waitpid(-1, &status, __WALL) > 0)
								;
							return 4;
						}
						s = get_regs(w);
						if (n < 400 || (s.pc >> 20) != last_hi) {
							fprintf(stderr, "  step %05d pc=0x%lx x0=0x%lx x1=0x%lx x8=0x%lx x9=0x%lx x19=0x%lx\n",
								n, s.pc, s.x[0], s.x[1], s.x[8], s.x[9], s.x[19]);
						}
						last_hi = s.pc >> 20;
					}
				}
				ptrace(PTRACE_CONT, w, 0, 0);
				continue;
			}
			ptrace(PTRACE_CONT, w, 0, 0);
			continue;
		}

		if (sig == SIGSEGV || sig == SIGBUS || sig == SIGABRT ||
		    sig == SIGFPE || sig == SIGILL) {
			siginfo_t si;
			struct user_pt pt;
			unsigned long fp;
			int i;

			memset(&si, 0, sizeof(si));
			ptrace(PTRACE_GETSIGINFO, w, 0, &si);
			pt = get_regs(w);
			fprintf(stderr, "crash-tracer: tid %d sig %d code %d addr %p\n",
				w, sig, si.si_code, si.si_addr);
			fprintf(stderr, "crash-tracer: pc=0x%lx lr=0x%lx fp(x29)=0x%lx x0=0x%lx x1=0x%lx x8=0x%lx\n",
				pt.pc, pt.x[30], pt.x[29], pt.x[0], pt.x[1], pt.x[8]);
			fprintf(stderr, "crash-tracer: backtrace (frame-pointer chain):\n");
			fp = pt.x[29];
			for (i = 0; i < 24 && fp; i++) {
				unsigned long next = peek(w, fp);
				unsigned long ret = peek(w, fp + 8);
				fprintf(stderr, "  #%02d pc=0x%lx (fp=0x%lx)\n", i, ret, fp);
				if (next <= fp || next - fp > 0x100000)
					break;
				fp = next;
			}
			/* dump the tracee's maps so the addresses can be symbolized */
			{
				char mp[48];
				FILE *mf;
				snprintf(mp, sizeof(mp), "/proc/%d/maps", w);
				mf = fopen(mp, "r");
				if (mf) {
					char line[512];
					fprintf(stderr, "crash-tracer: --- maps ---\n");
					while (fgets(line, sizeof(line), mf))
						fputs(line, stderr);
					fclose(mf);
				}
			}
			kill(pid, SIGKILL);
			while (waitpid(-1, &status, __WALL) > 0)
				;
			return 128 + sig;
		}

		/* swallow ptrace events and signal-delivery-stops we don't care
		 * about (bionic installs handlers for lots of signals) */
		ptrace(PTRACE_CONT, w, 0, (sig == SIGTRAP || sig == SIGCHLD ||
			sig == SIGPIPE) ? 0 : sig);
	}
}
