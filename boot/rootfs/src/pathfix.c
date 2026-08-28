/* pathfix.c — LD_PRELOAD: rewrite legacy /system/vendor paths (M7).
 *
 * Vendor daemons from the pre-Treble-merge era open configs at
 * /system/vendor/... (stock: /system/vendor symlinks to /vendor). On
 * AginxOS /system_a is a dm-verity read-only mount and has no such
 * symlink, so those opens fail — observed 2026-08-29 with netmgrd
 * aborting on /system/vendor/etc/data/netmgr_config.xml. This preload
 * rewrites the prefix before the real call. Default:
 *   /system/vendor/ -> /vendor/
 * Override with PATHFIX="from:to" (single pair).
 * Build: NDK clang -shared. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#include <stdarg.h>

static const char *g_from = "/system/vendor/";
static const char *g_to = "/vendor/";

static void init_paths(void)
{
	const char *e = getenv("PATHFIX");
	char *c;
	if (!e)
		return;
	c = strchr(e, ':');
	if (c && c > e && c[1]) {
		static char from[256], to[256];
		size_t n = (size_t)(c - e);
		if (n < sizeof(from) && strlen(c + 1) < sizeof(to)) {
			memcpy(from, e, n); from[n] = 0;
			strcpy(to, c + 1);
			g_from = from; g_to = to;
		}
	}
}

/* Returns a rewritten path in buf (always NUL-terminated) or NULL when
 * the prefix doesn't match and the original should be used as-is. */
static const char *fix(const char *path, char *buf, size_t bufsz)
{
	size_t fl;
	if (!path)
		return NULL;
	fl = strlen(g_from);
	if (strncmp(path, g_from, fl) != 0)
		return NULL;
	if (strlen(g_to) + strlen(path + fl) + 1 > bufsz)
		return NULL;
	strcpy(buf, g_to);
	strcat(buf, path + fl);
	return buf;
}

#define FIX1(call, path, ...) do { \
	char b[512]; const char *p = fix(path, b, sizeof(b)); \
	return real_##call(p ? p : path, ##__VA_ARGS__); \
} while (0)

static int (*real_open)(const char *, int, ...);
static int (*real_openat)(int, const char *, int, ...);
static FILE *(*real_fopen)(const char *, const char *);
static FILE *(*real_fopen64)(const char *, const char *);
static DIR *(*real_opendir)(const char *);
static int (*real_stat)(const char *, struct stat *);
static int (*real_access)(const char *, int);

#define INIT(sym) do { if (!real_##sym) real_##sym = dlsym(RTLD_NEXT, #sym); } while (0)

int open(const char *path, int flags, ...)
{
	mode_t m = 0;
	if (flags & O_CREAT) {
		va_list ap; va_start(ap, flags);
		m = va_arg(ap, int); va_end(ap);
	}
	INIT(open); init_paths();
	{
		char b[512]; const char *p = fix(path, b, sizeof(b));
		return real_open(p ? p : path, flags, m);
	}
}

int openat(int dirfd, const char *path, int flags, ...)
{
	mode_t m = 0;
	if (flags & O_CREAT) {
		va_list ap; va_start(ap, flags);
		m = va_arg(ap, int); va_end(ap);
	}
	INIT(openat); init_paths();
	{
		char b[512]; const char *p = fix(path, b, sizeof(b));
		return real_openat(dirfd, p ? p : path, flags, m);
	}
}

FILE *fopen(const char *path, const char *mode)
{
	INIT(fopen); init_paths();
	FIX1(fopen, path, mode);
}

FILE *fopen64(const char *path, const char *mode)
{
	INIT(fopen64); init_paths();
	FIX1(fopen64, path, mode);
}

DIR *opendir(const char *path)
{
	INIT(opendir); init_paths();
	FIX1(opendir, path);
}

int stat(const char *path, struct stat *st)
{
	INIT(stat); init_paths();
	FIX1(stat, path, st);
}

int access(const char *path, int mode)
{
	INIT(access); init_paths();
	FIX1(access, path, mode);
}
