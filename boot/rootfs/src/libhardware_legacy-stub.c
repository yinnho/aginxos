/* libhardware_legacy-stub.c — minimal wake-lock for netmgrd (M7).
 *
 * libhardware_legacy.so is absent from every partition we mount on
 * redfin, and libnetmgr.so's only imports from it are the two wakelock
 * calls. Implement them the way the real lib does: write the lock name
 * to /sys/power/wake_lock / wake_unlock. netmgrd holds a wakelock
 * while a data call is up; the no-op return would also satisfy the
 * linker, but doing it for real costs nothing.
 * Build: NDK clang -shared. */
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

static int wl_write(const char *path, const char *id)
{
	int fd, n;

	if (!id)
		return -1;
	fd = open(path, O_WRONLY | O_CLOEXEC);
	if (fd < 0)
		return -1;
	n = (int)strlen(id);
	if (write(fd, id, (size_t)n) != n)
		n = -1;
	close(fd);
	return n < 0 ? -1 : 0;
}

int acquire_wake_lock(const char *id, const char *unused)
{
	(void)unused;
	return wl_write("/sys/power/wake_lock", id);
}

int release_wake_lock(const char *id, const char *unused)
{
	(void)unused;
	return wl_write("/sys/power/wake_unlock", id);
}
