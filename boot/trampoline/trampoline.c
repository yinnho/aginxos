/*
 * AginxOS early entry (rdinit). Must stay tiny and boring.
 * Proven on Pixel 5 (redfin): execve stock first_stage → Android boots.
 *
 * Optional:
 *   /aginxos/hold   — stay here forever (bring-up)
 *   /aginxos/splash — run /aginxos/aginxos-init splash-test then continue
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <sys/wait.h>
#include <unistd.h>
#include <string.h>

static void kmsg(const char *s) {
  int fd = open("/dev/kmsg", O_WRONLY | O_CLOEXEC);
  if (fd < 0)
    return;
  write(fd, s, strlen(s));
  close(fd);
}

static int exists(const char *p) { return access(p, F_OK) == 0; }

static void run_splash(char **envp) {
  if (!exists("/aginxos/splash") || !exists("/aginxos/aginxos-init"))
    return;
  kmsg("aginxos-trampoline: splash child\n");
  pid_t pid = fork();
  if (pid == 0) {
    char *argv[] = {"/aginxos/aginxos-init", "splash-test", NULL};
    execve(argv[0], argv, envp);
    _exit(127);
  }
  if (pid > 0) {
    int st = 0;
    waitpid(pid, &st, 0);
  }
}

int main(int argc, char **argv, char **envp) {
  (void)argc;
  (void)argv;
  kmsg("aginxos-trampoline: start\n");

  if (exists("/aginxos/hold")) {
    kmsg("aginxos-trampoline: HOLD\n");
    for (;;)
      pause();
  }

  run_splash(envp);

  kmsg("aginxos-trampoline: exec first_stage\n");
  {
    char *a[] = {"/init", NULL};
    execve("/aginxos/first_stage_init", a, envp);
  }
  kmsg("aginxos-trampoline: execve failed\n");
  for (;;)
    pause();
  return 1;
}
