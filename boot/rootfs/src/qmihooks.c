/* qmihooks.c — LD_PRELOAD: log vendor libqmi_cci entry points (M7).
 *
 * netmgrd exits silently with main()==1 after NetmgrNetdClientInit
 * (2026-08-29) and none of the public source mirrors match the
 * SM7250 binary's control flow. These hooks forward-and-log the
 * libqmi_cci calls (service-object getters, init_instance,
 * send_msg_sync/async, release) so the exact init step that never
 * returns is visible on stderr, with the service and message id
 * resolved by tracking the *_get_service_object_internal_v01
 * pointers. NB: libqmi_cci's QRTR transport writes with write(2)/
 * sendmmsg — NOT sendto/sendmsg — so sock-trace.so cannot see these
 * frames; this preload is the QMI visibility layer instead.
 * Build: NDK clang -shared. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>

static void logf_(const char *fmt, ...)
{
	va_list ap;
	va_start(ap, fmt);
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	fflush(stderr);
}

/* service-object pointer → name, filled by the getters */
#define MAXOBJS 16
static const void *objs[MAXOBJS];
static const char *names[MAXOBJS];
static int nobjs;

static const char *svcname(const void *o)
{
	for (int i = 0; i < nobjs; i++)
		if (objs[i] == o)
			return names[i];
	return "?";
}


static int (*real_init)(void *, unsigned, void *, void *, void *, unsigned, void **);
int qmi_client_init_instance(void *obj, unsigned conn, void *ind_cb,
			     void *notify_cb, void *os, unsigned timeout,
			     void **clnt)
{
	if (!real_init) real_init = dlsym(RTLD_NEXT, "qmi_client_init_instance");
	logf_("[Q] init_instance svc=%p conn=%u\n", (const char*)obj, conn);
	int r = real_init(obj, conn, ind_cb, notify_cb, os, timeout, clnt);
	logf_("[Q] init_instance svc=%p rc=%d clnt=%p\n", (const char*)obj, r,
	      clnt ? *clnt : 0);
	return r;
}

static int (*real_sync)(void *, unsigned, void *, unsigned, void *, unsigned, unsigned);
int qmi_client_send_msg_sync(void *clnt, unsigned id, void *req,
			     unsigned reqlen, void *resp, unsigned resplen,
			     unsigned timeout)
{
	if (!real_sync) real_sync = dlsym(RTLD_NEXT, "qmi_client_send_msg_sync");
	logf_("[Q] send_sync clnt=%p msg=0x%02x len=%u\n", clnt, id, reqlen);
	int r = real_sync(clnt, id, req, reqlen, resp, resplen, timeout);
	logf_("[Q] send_sync clnt=%p msg=0x%02x rc=%d\n", clnt, id, r);
	return r;
}

static void *(*real_async)(void *, unsigned, void *, unsigned, void *, unsigned, void *, void *);
void *qmi_client_send_msg_async(void *clnt, unsigned id, void *req,
				unsigned reqlen, void *resp, unsigned resplen,
				void *cb, void *cbdata)
{
	if (!real_async) real_async = dlsym(RTLD_NEXT, "qmi_client_send_msg_async");
	logf_("[Q] send_async clnt=%p msg=0x%02x len=%u\n", clnt, id, reqlen);
	void *t = real_async(clnt, id, req, reqlen, resp, resplen, cb, cbdata);
	logf_("[Q] send_async clnt=%p msg=0x%02x txn=%p\n", clnt, id, t);
	return t;
}

static void (*real_release)(void *);
void qmi_client_release(void *clnt)
{
	if (!real_release) real_release = dlsym(RTLD_NEXT, "qmi_client_release");
	logf_("[Q] release clnt=%p\n", clnt);
	real_release(clnt);
}
