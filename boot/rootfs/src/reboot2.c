/* reboot2 <reason> — invoke SYS_reboot RESTART2 with a reason string,
 * e.g. `reboot2 bootloader` == what `adb reboot bootloader` does on Android.
 * Qualcomm ABL reads the restart reason and enters fastboot for "bootloader". */
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <linux/reboot.h>

int main(int argc, char **argv)
{
	if (argc < 2) {
		fprintf(stderr, "usage: %s <reason>\n", argv[0]);
		return 2;
	}
	sync();
	long r = syscall(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
			 LINUX_REBOOT_CMD_RESTART2, argv[1]);
	if (r)
		perror("reboot");
	return r != 0;
}
