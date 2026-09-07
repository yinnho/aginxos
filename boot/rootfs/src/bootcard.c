/* bootcard.c — AginxOS boot screen, Matrix edition (M5 card → 开机体验④).
 *
 * While the bring-up chain reports into /run/boot.state:
 *
 *     <key> <ok|fail|run> [detail ...]
 *
 * the panel plays Matrix code-rain (green glyph trails falling in
 * columns), with the latest event echoed in a dim footer line. When
 * "done ok" lands and every stage is OK, the rain cuts to black and a
 * green terminal typewrites: "Wake up..." → "The Matrix has you..." →
 * the AginxOS wordmark — complete ~4 s in, because aginx-term-handoff
 * kills this process ~5-7 s after the done line (2 s poll + sleep 5) and
 * hands the panel to aginx-term (voice face). A failed boot keeps
 * raining under a red "BOOT STOPPED" verdict.
 *
 * It is the ONLY thing that ever holds DRM master once it starts; exiting
 * would drop master and dsi_backlight's dpms hooks would blank the panel,
 * so on device it never exits on its own.
 *
 * DRM path is the splash2 skeleton (probe connector -> mode[0] -> encoder ->
 * possible_crtcs -> dumb fb -> SETCRTC) with its msm_drm 4.19 quirks intact:
 * zero count_fbs/encoders on the second GETRESOURCES, skip connectors
 * without a bound encoder or modes, prefer DSI (type 16).
 *
 * Host verification: `bootcard --ppm out.ppm [statefile] [frame]` renders
 * one frame into a P6 PPM instead of touching DRM — rain at frame 3 by
 * default, or the END sequence (frame 30 = fully typed) when the state
 * file shows done ok. Unseeded rand() keeps host frames deterministic.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

static void kmsg(const char *s) {
  int fd = open("/dev/kmsg", O_WRONLY | O_CLOEXEC);
  if (fd < 0) return;
  write(fd, s, strlen(s));
  close(fd);
}
static void kmsgf(const char *fmt, ...) {
  char b[160];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(b, sizeof b, fmt, ap);
  va_end(ap);
  kmsg(b);
}

/* ---------------- DRM (raw ioctls, no headers needed) ---------------- */
#ifndef __linux__   /* host PPM build: synthesize the Linux ioctl encoding */
#undef _IOC
#undef _IO
#undef _IOC_READ
#undef _IOC_WRITE
#define _IOC_READ  2u
#define _IOC_WRITE 1u
#define _IOC(d, t, nr, sz) (((d) << 30) | ((sz) << 16) | ((t) << 8) | (nr))
#define _IO(t, nr) _IOC(0u, (t), (nr), 0)
#endif
#define DRM_IOCTL_BASE 'd'
#define DRM_IOWR(nr, type) _IOC(_IOC_READ|_IOC_WRITE, DRM_IOCTL_BASE, (nr), sizeof(type))
#define DRM_IO(nr) _IO(DRM_IOCTL_BASE, (nr))

struct drm_mode_card_res {
  uint64_t fb_id_ptr, crtc_id_ptr, connector_id_ptr, encoder_id_ptr;
  uint32_t count_fbs, count_crtcs, count_connectors, count_encoders;
  uint32_t min_width, max_width, min_height, max_height;
};
struct drm_mode_modeinfo {
  uint32_t clock;
  uint16_t hdisplay, hsync_start, hsync_end, htotal, hskew;
  uint16_t vdisplay, vsync_start, vsync_end, vtotal, vscan;
  uint32_t vrefresh, flags, type;
  char name[32];
};
struct drm_mode_crtc {
  uint64_t set_connectors_ptr;
  uint32_t count_connectors, crtc_id, fb_id, x, y, gamma_size, mode_valid;
  struct drm_mode_modeinfo mode;
};
struct drm_mode_get_connector {
  uint64_t encoders_ptr, modes_ptr, props_ptr, prop_values_ptr;
  int count_modes, count_props, count_encoders;
  uint32_t encoder_id, connector_id, connector_type, connector_type_id;
  uint32_t pad;
};
struct drm_mode_get_encoder {
  uint32_t encoder_id, encoder_type, crtc_id;
  uint32_t possible_crtcs, possible_clones;
};
struct drm_mode_create_dumb {
  uint32_t height, width, bpp, flags, handle, pitch;
  uint64_t size;
};
struct drm_mode_map_dumb { uint32_t handle, pad; uint64_t offset; };
struct drm_mode_fb_cmd2 {
  uint32_t fb_id, width, height, pixel_format, flags;
  uint32_t handles[4], pitches[4], offsets[4];
  uint64_t modifier[4];
};
#define DRM_IOCTL_MODE_GETRESOURCES DRM_IOWR(0xA0, struct drm_mode_card_res)
#define DRM_IOCTL_MODE_SETCRTC DRM_IOWR(0xA2, struct drm_mode_crtc)
#define DRM_IOCTL_MODE_GETENCODER DRM_IOWR(0xA6, struct drm_mode_get_encoder)
#define DRM_IOCTL_MODE_GETCONNECTOR DRM_IOWR(0xA7, struct drm_mode_get_connector)
#define DRM_IOCTL_MODE_CREATE_DUMB DRM_IOWR(0xB2, struct drm_mode_create_dumb)
#define DRM_IOCTL_MODE_MAP_DUMB DRM_IOWR(0xB3, struct drm_mode_map_dumb)
#define DRM_IOCTL_MODE_ADDFB2 DRM_IOWR(0xB8, struct drm_mode_fb_cmd2)
struct drm_mode_crtc_page_flip {
  uint32_t fb_id, crtc_id, flags, reserved;
  uint64_t user_data;
};
#define DRM_IOCTL_MODE_PAGE_FLIP DRM_IOWR(0xB0, struct drm_mode_crtc_page_flip)
#define DRM_IOCTL_SET_MASTER DRM_IO(0x1e)
#define DRM_FORMAT_XRGB8888 0x34325258u

/* ---------------- framebuffer ---------------- */
static uint32_t *pix;      /* XRGB8888 */
static uint32_t pitch_px;  /* pixels per line */
static uint32_t fb_w, fb_h;

#define C_BG      0x00020503u   /* near-black, faint green cast (Matrix) */
#define C_WHITE   0x00F5F7FAu
#define C_ACCENT  0x0034D399u   /* emerald — "OS" in the wordmark */
#define C_GREEN   0x0000FF41u   /* Matrix green — typewriter lines */
#define C_FAIL    0x00EF4444u
#define C_RAINHEAD 0x00C8FFC8u  /* rain head: near-white green */
#define C_RAINDIM  0x00005A20u  /* footer status line */

/* ---------------- 5x8 string-art font ----------------
 * Each glyph is 8 rows of 5 chars ('#'=on, anything else=off), parsed once
 * at startup. Caps sit rows 1-7, lowercase body rows 3-7, descenders row 8.
 * String art in the source keeps the shapes reviewable in place.
 */
static const struct { char c; const char *s; } GLYPHS[] = {
{'A', ".###.\n#...#\n#...#\n#####\n#...#\n#...#\n#...#\n....."},
{'B', "####.\n#...#\n#...#\n####.\n#...#\n#...#\n####.\n....."},
{'C', ".###.\n#...#\n#....\n#....\n#....\n#...#\n.###.\n....."},
{'D', "####.\n#...#\n#...#\n#...#\n#...#\n#...#\n####.\n....."},
{'E', "#####\n#....\n#....\n####.\n#....\n#....\n#####\n....."},
{'F', "#####\n#....\n#....\n####.\n#....\n#....\n#....\n....."},
{'G', ".###.\n#...#\n#....\n#.###\n#...#\n#...#\n.###.\n....."},
{'H', "#...#\n#...#\n#...#\n#####\n#...#\n#...#\n#...#\n....."},
{'I', ".###.\n..#..\n..#..\n..#..\n..#..\n..#..\n.###.\n....."},
{'J', "..###\n...#.\n...#.\n...#.\n...#.\n#..#.\n.##..\n....."},
{'K', "#...#\n#..#.\n#.#..\n##...\n#.#..\n#..#.\n#...#\n....."},
{'L', "#....\n#....\n#....\n#....\n#....\n#....\n#####\n....."},
{'M', "#...#\n##.##\n#.#.#\n#.#.#\n#...#\n#...#\n#...#\n....."},
{'N', "#...#\n##..#\n##..#\n#.#.#\n#..##\n#..##\n#...#\n....."},
{'O', ".###.\n#...#\n#...#\n#...#\n#...#\n#...#\n.###.\n....."},
{'P', "####.\n#...#\n#...#\n####.\n#....\n#....\n#....\n....."},
{'Q', ".###.\n#...#\n#...#\n#...#\n#.#.#\n#..#.\n.##.#\n....."},
{'R', "####.\n#...#\n#...#\n####.\n#.#..\n#..#.\n#...#\n....."},
{'S', ".####\n#....\n#....\n.###.\n....#\n....#\n####.\n....."},
{'T', "#####\n..#..\n..#..\n..#..\n..#..\n..#..\n..#..\n....."},
{'U', "#...#\n#...#\n#...#\n#...#\n#...#\n#...#\n.###.\n....."},
{'V', "#...#\n#...#\n#...#\n#...#\n#...#\n.#.#.\n..#..\n....."},
{'W', "#...#\n#...#\n#...#\n#.#.#\n#.#.#\n##.##\n#...#\n....."},
{'X', "#...#\n#...#\n.#.#.\n..#..\n.#.#.\n#...#\n#...#\n....."},
{'Y', "#...#\n#...#\n.#.#.\n..#..\n..#..\n..#..\n..#..\n....."},
{'Z', "#####\n....#\n...#.\n..#..\n.#...\n#....\n#####\n....."},
{'a', ".....\n.....\n.###.\n....#\n.####\n#...#\n.####\n....."},
{'b', "#....\n#....\n####.\n#...#\n#...#\n#...#\n####.\n....."},
{'c', ".....\n.....\n.###.\n#....\n#....\n#....\n.###.\n....."},
{'d', "....#\n....#\n.####\n#...#\n#...#\n#...#\n.####\n....."},
{'e', ".....\n.....\n.###.\n#...#\n#####\n#....\n.###.\n....."},
{'f', "..##.\n.#..#\n.#...\n###..\n.#...\n.#...\n.#...\n....."},
{'g', ".....\n.....\n.####\n#...#\n#...#\n.####\n....#\n.###."},
{'h', "#....\n#....\n####.\n#...#\n#...#\n#...#\n#...#\n....."},
{'i', "..#..\n.....\n..#..\n..#..\n..#..\n..#..\n..#..\n....."},
{'j', "...#.\n.....\n...#.\n...#.\n...#.\n...#.\n#..#.\n.##.."},
{'k', "#....\n#....\n#..#.\n#.#..\n##...\n#.#..\n#..#.\n....."},
{'l', ".##..\n..#..\n..#..\n..#..\n..#..\n..#..\n.###.\n....."},
{'m', ".....\n.....\n##.#.\n#.#.#\n#.#.#\n#.#.#\n#.#.#\n....."},
{'n', ".....\n.....\n####.\n#...#\n#...#\n#...#\n#...#\n....."},
{'o', ".....\n.....\n.###.\n#...#\n#...#\n#...#\n.###.\n....."},
{'p', ".....\n.....\n####.\n#...#\n#...#\n####.\n#....\n#...."},
{'q', ".....\n.....\n.####\n#...#\n#...#\n.####\n....#\n....#"},
{'r', ".....\n.....\n.####\n#...#\n#....\n#....\n#....\n....."},
{'s', ".....\n.....\n.####\n#....\n.###.\n....#\n####.\n....."},
{'t', ".#...\n.#...\n###..\n.#...\n.#...\n.#..#\n..##.\n....."},
{'u', ".....\n.....\n#...#\n#...#\n#...#\n#...#\n.####\n....."},
{'v', ".....\n.....\n#...#\n#...#\n#...#\n.#.#.\n..#..\n....."},
{'w', ".....\n.....\n#...#\n#...#\n#.#.#\n#.#.#\n.#.#.\n....."},
{'x', ".....\n.....\n#...#\n.#.#.\n..#..\n.#.#.\n#...#\n....."},
{'y', ".....\n.....\n#...#\n#...#\n#...#\n.####\n....#\n.###."},
{'z', ".....\n.....\n#####\n...#.\n..#..\n.#...\n#####\n....."},
{'0', ".###.\n#...#\n#..##\n#.#.#\n##..#\n#...#\n.###.\n....."},
{'1', "..#..\n.##..\n..#..\n..#..\n..#..\n..#..\n.###.\n....."},
{'2', ".###.\n#...#\n....#\n...#.\n..#..\n.#...\n#####\n....."},
{'3', "####.\n....#\n....#\n.###.\n....#\n....#\n####.\n....."},
{'4', "...#.\n..##.\n.#.#.\n#..#.\n#####\n...#.\n...#.\n....."},
{'5', "#####\n#....\n####.\n....#\n....#\n#...#\n.###.\n....."},
{'6', "..##.\n.#...\n#....\n####.\n#...#\n#...#\n.###.\n....."},
{'7', "#####\n....#\n...#.\n..#..\n..#..\n..#..\n..#..\n....."},
{'8', ".###.\n#...#\n#...#\n.###.\n#...#\n#...#\n.###.\n....."},
{'9', ".###.\n#...#\n#...#\n.####\n....#\n...#.\n.##..\n....."},
{' ', ".....\n.....\n.....\n.....\n.....\n.....\n.....\n....."},
{'.', ".....\n.....\n.....\n.....\n.....\n.##..\n.##..\n....."},
{',', ".....\n.....\n.....\n.....\n.....\n.##..\n.##..\n.#..."},
{':', ".....\n.....\n.##..\n.##..\n.....\n.##..\n.##..\n....."},
{';', ".....\n.....\n.##..\n.##..\n.....\n.##..\n.##..\n.#..."},
{'/', "....#\n....#\n...#.\n...#.\n..#..\n.#...\n.#...\n#...."},
{'-', ".....\n.....\n.....\n.....\n.###.\n.....\n.....\n....."},
{'_', ".....\n.....\n.....\n.....\n.....\n.....\n.....\n#####"},
{'(', "..#..\n.#...\n.#...\n#....\n#....\n.#...\n.#...\n..#.."},
{')', "..#..\n...#.\n...#.\n....#\n....#\n...#.\n...#.\n..#.."},
{'+', ".....\n.....\n..#..\n..#..\n#####\n..#..\n..#..\n....."},
{'=', ".....\n.....\n.....\n#####\n.....\n#####\n.....\n....."},
{'!', "..#..\n..#..\n..#..\n..#..\n..#..\n.....\n..#..\n....."},
{'?', ".###.\n#..#.\n...#.\n..#..\n..#..\n.....\n..#..\n....."},
{'\'', "..#..\n..#..\n.....\n.....\n.....\n.....\n.....\n....."},
{'%', "#...#\n#..#.\n...#.\n..#..\n.#...\n#..#.\n#...#\n....."},
{'<', "....#\n...#.\n..#..\n.#...\n..#..\n...#.\n....#\n....."},
{'>', "#....\n.#...\n..#..\n...#.\n..#..\n.#...\n#....\n....."},
};

static unsigned char fontbits[128][8];  /* [row] bit4..bit0 = col left->right */
static void font_init(void) {
  for (unsigned g = 0; g < sizeof GLYPHS / sizeof GLYPHS[0]; g++) {
    int c = GLYPHS[g].c & 127, row = 0, col = 0;
    for (const char *p = GLYPHS[g].s; *p && row < 8; p++) {
      if (*p == '\n') { row++; col = 0; continue; }
      if (col < 5 && *p == '#') fontbits[c][row] |= 0x10 >> col;
      col++;
    }
  }
}

/* ---------------- drawing primitives ---------------- */
static void fill_rect(int x, int y, int w, int h, uint32_t c) {
  if (w <= 0 || h <= 0) return;
  if (x < 0) { w += x; x = 0; }
  if (y < 0) { h += y; y = 0; }
  if (x + w > (int)fb_w) w = fb_w - x;
  if (y + h > (int)fb_h) h = fb_h - y;
  for (int j = 0; j < h; j++) {
    uint32_t *r = pix + (y + j) * pitch_px + x;
    for (int i = 0; i < w; i++) r[i] = c;
  }
}
static int text_w(const char *s, int scale) {
  return ((int)strlen(s) * 6 - 1) * scale;
}
static int draw_text(int x, int y, const char *s, int scale, uint32_t c) {
  for (; *s; s++, x += 6 * scale) {
    unsigned char *g = fontbits[*s & 127];
    for (int r = 0; r < 8; r++)
      for (int col = 0; col < 5; col++)
        if (g[r] & (0x10 >> col))
          fill_rect(x + col * scale, y + r * scale, scale, scale, c);
  }
  return x;
}

static void draw_glyph(int x, int y, char c, int scale, uint32_t col) {
  unsigned char *g = fontbits[(unsigned char)c & 127];
  for (int r = 0; r < 8; r++)
    for (int k = 0; k < 5; k++)
      if (g[r] & (0x10 >> k))
        fill_rect(x + k * scale, y + r * scale, scale, scale, col);
}

/* ---------------- boot state ---------------- */
#define NKEYS 10
static const char *KEYS[NKEYS] = {
  "kernel", "rootfs", "display", "touch", "battery",
  "modem", "wlan", "wifi", "dhcp", "internet",
};
static const char *LABELS[NKEYS] = {
  "KERNEL", "ROOTFS", "DISPLAY", "TOUCH", "BATTERY",
  "MODEM", "WLAN", "WIFI", "DHCP", "INTERNET",
};
enum { ST_PEND = 0, ST_RUN, ST_OK, ST_FAIL };
static int st_status[NKEYS];
static char st_detail[NKEYS][80];
static int done_ok, done_seen;
static char latest_ev[48];   /* rain footer: "key val" of the newest event */

struct snapshot { int status[NKEYS]; char detail[NKEYS][80]; int d[2]; };
static int read_state(const char *path) {
  struct snapshot before, after;
  memset(&before, 0, sizeof before);
  memcpy(before.status, st_status, sizeof st_status);
  memcpy(before.detail, st_detail, sizeof st_detail);
  before.d[0] = done_ok; before.d[1] = done_seen;

  FILE *f = fopen(path, "r");
  if (f) {
    char line[256];
    while (fgets(line, sizeof line, f)) {
      char key[32], val[32], det[80];
      int n = sscanf(line, "%31s %31s %79[^\n]", key, val, det);
      if (n < 2) continue;
      char *d = det;
      while (*d == ' ' || *d == '\t') d++;
      if (!strcmp(key, "done")) {
        done_seen = 1;
        if (!strcmp(val, "ok")) done_ok = 1;
        continue;
      }
      for (int i = 0; i < NKEYS; i++)
        if (!strcmp(key, KEYS[i])) {
          snprintf(latest_ev, sizeof latest_ev, "%s %s", key, val);
          st_status[i] = !strcmp(val, "ok") ? ST_OK
                       : !strcmp(val, "fail") ? ST_FAIL
                       : !strcmp(val, "run") ? ST_RUN : ST_PEND;
          if (n >= 3) {
            strncpy(st_detail[i], d, sizeof st_detail[i] - 1);
            st_detail[i][sizeof st_detail[i] - 1] = 0;
          }
        }
    }
    fclose(f);
  }
  memset(&after, 0, sizeof after);
  memcpy(after.status, st_status, sizeof st_status);
  memcpy(after.detail, st_detail, sizeof st_detail);
  after.d[0] = done_ok; after.d[1] = done_seen;
  return memcmp(&before, &after, sizeof before) != 0;
}

/* ---------------- Matrix rain + typewriter END ----------------
 * 开机体验④. END budget: aginx-term-handoff kills us ~5-7 s after
 * "done ok" lands (2 s poll + fixed sleep 5); the sequence below
 * completes at ~4 s so the wordmark holds before the voice face
 * takes the panel. Ticks are the main loop's ~90 ms frames.
 */
#define CELL      24   /* rain glyph advance: 6 px * scale 4 */
#define GLYPH_H   32   /* rain glyph height: 8 px * scale 4 */
#define TRAIL     28   /* cells per column trail (~40% of panel, film streak) */
#define RAIN_MAX  64
static const char RAINCS[] =
    "01<>+-/ABCDEFGHKLMNPRSTUVWXYZ23456789abcdefghkmnoprstuvwxyz";
static int head_y[RAIN_MAX];
static int spd[RAIN_MAX];
static char cells[RAIN_MAX][TRAIL];
static uint32_t rain_trail[TRAIL];
static int rain_ready;
static long long end_frame = -1;

static int rain_speed(void) {
  return CELL * (2 + rand() % 3) - CELL / 4;   /* 42/66/90 px, uniform step */
}

static void rain_init(int seed) {
  if (rain_ready) return;
  rain_ready = 1;
  if (seed) srand((unsigned)(time(NULL) ^ getpid()));
  /* film fade: bright head, linear falloff to bg (the reference rain:
   * RGB(0, 255-13j, 0) per glyph down the trail) */
  for (int k = 0; k < TRAIL; k++) {
    int t = 255 * (TRAIL - 1 - k) / TRAIL;   /* head k=0: full green */
    rain_trail[k] = (uint32_t)(
        ((0x02 * (255 - t) + 0x00 * t) / 255) << 16 |
        ((0x05 * (255 - t) + 0xFF * t) / 255) << 8 |
        ((0x03 * (255 - t) + 0x30 * t) / 255));
  }
  int n = (int)(fb_w / CELL);
  if (n > RAIN_MAX) n = RAIN_MAX;
  for (int i = 0; i < n; i++) {
    /* heads already spread across (and just above) the panel: the very
     * first frame reads as ongoing rain, not an empty screen filling up */
    head_y[i] =
        (int)((unsigned)rand() % (fb_h + TRAIL * GLYPH_H)) - TRAIL * GLYPH_H;
    /* reference rain moves 5-9 px per 20 ms tick (crosses in 1.4-2.6 s);
     * scaled to this panel (3.7x taller) at our ~60 ms frame: 1.5-3.2
     * cells per frame, constant step = uniform motion */
    spd[i] = rain_speed();
    for (int k = 0; k < TRAIL; k++)
      cells[i][k] = RAINCS[rand() % (sizeof RAINCS - 1)];
  }
}

static void render_rain(long tsec) {
  int n = (int)(fb_w / CELL);
  if (n > RAIN_MAX) n = RAIN_MAX;
  for (int i = 0; i < n; i++) {
    head_y[i] += spd[i];
    if (head_y[i] - TRAIL * GLYPH_H > (int)fb_h) {
      head_y[i] = -(int)((unsigned)rand() % (fb_h / 2)) - GLYPH_H;
      spd[i] = rain_speed();
    }
    if (rand() % 5 == 0)
      cells[i][rand() % TRAIL] = RAINCS[rand() % (sizeof RAINCS - 1)];
    for (int k = 0; k < TRAIL; k++) {
      int y = head_y[i] - k * GLYPH_H;
      if (y + GLYPH_H < 0 || y > (int)fb_h) continue;
      draw_glyph(i * CELL, y, cells[i][k], 4,
                 k == 0 ? C_RAINHEAD : rain_trail[k]);
    }
  }
  /* footer: wall clock + latest boot event (key val only — no detail,
   * detail is free text and must never put anything secret on the panel) */
  char ft[16];
  snprintf(ft, sizeof ft, "T+%02ld:%02ld", tsec / 60, tsec % 60);
  draw_text(40, fb_h - 64, ft, 2, C_RAINDIM);
  if (latest_ev[0])
    draw_text(fb_w - 40 - text_w(latest_ev, 2), fb_h - 64, latest_ev, 2,
              C_RAINDIM);
}

static int boot_success(void) {
  if (!done_ok || !done_seen) return 0;
  for (int i = 0; i < NKEYS; i++)
    if (st_status[i] != ST_OK) return 0;
  return 1;
}

/* chars revealed by tick: 1 per ~90 ms tick */
static int typed(uint64_t ticks, uint64_t start, int total) {
  if (ticks <= start) return 0;
  uint64_t n = ticks - start;
  return n >= (uint64_t)total ? total : (int)n;
}

static void draw_text_n(int x, int y, const char *s, int n, int scale,
                        uint32_t col) {
  for (int i = 0; i < n && s[i]; i++, x += 6 * scale)
    draw_glyph(x, y, s[i], scale, col);
}

static void render_end(uint64_t ticks) {
  static const char L1[] = "Wake up...";
  static const char L2[] = "The Matrix has you...";
  static const char WM[] = "AginxOS";
  int cs = 7;
  int y1 = fb_h * 36 / 100;
  int y2 = y1 + 8 * cs + 44;
  int l1n = (int)sizeof L1 - 1, l2n = (int)sizeof L2 - 1;

  uint64_t t2 = l1n + 3;                              /* line 1 pause end */
  uint64_t t3 = t2 + l2n + 3;                         /* line 2 pause end */

  int n1 = typed(ticks, 0, l1n);
  int x1 = (fb_w - text_w(L1, cs)) / 2;
  draw_text_n(x1, y1, L1, n1, cs, C_GREEN);
  if (n1 > 0 && ticks < t2) {
    int cx = x1 + (n1 < l1n ? n1 * 6 * cs : text_w(L1, cs));
    fill_rect(cx, y1, 5 * cs, 8 * cs, C_GREEN);
  }

  int n2 = typed(ticks, t2, l2n);
  int x2 = (fb_w - text_w(L2, cs)) / 2;
  draw_text_n(x2, y2, L2, n2, cs, C_GREEN);
  if (n2 > 0 && ticks >= t2 && ticks < t3) {
    int cx = x2 + (n2 < l2n ? n2 * 6 * cs : text_w(L2, cs));
    fill_rect(cx, y2, 5 * cs, 8 * cs, C_GREEN);
  }

  int n3 = typed(ticks, t3, (int)sizeof WM - 1);
  if (n3 > 0) {
    int ws = 13;
    int ww = text_w("Aginx", ws) + 6 * ws + text_w("OS", ws);
    int wx = (fb_w - ww) / 2;
    int wy = fb_h * 56 / 100;
    int wmn = (int)sizeof WM - 1;
    for (int i = 0; i < n3; i++)
      draw_glyph(wx + i * 6 * ws, wy, WM[i], ws, i < 5 ? C_WHITE : C_ACCENT);
    if (ticks >= t3) {
      int cx = wx + (n3 < wmn ? n3 * 6 * ws : text_w(WM, ws));
      /* solid while typing, breathing after the wordmark lands */
      if (n3 < wmn || (ticks / 5) % 2 == 0)
        fill_rect(cx, wy, 5 * ws, 8 * ws, C_GREEN);
    }
  }
}

static void render_stopped(void) {
  const char *l1 = "BOOT STOPPED";
  int s1 = 6;
  draw_text((fb_w - text_w(l1, s1)) / 2, fb_h * 42 / 100, l1, s1, C_FAIL);
  char l2[96] = "INCOMPLETE";
  for (int i = 0; i < NKEYS; i++)
    if (st_status[i] == ST_FAIL) {
      snprintf(l2, sizeof l2, "%s FAILED%s%s", LABELS[i],
               st_detail[i][0] ? " - " : "", st_detail[i]);
      break;
    }
  int s2 = 3, maxw = fb_w - 160;
  while (text_w(l2, s2) > maxw && strlen(l2) > 1) l2[strlen(l2) - 1] = 0;
  draw_text((fb_w - text_w(l2, s2)) / 2, fb_h * 42 / 100 + 8 * s1 + 40, l2,
            s2, C_FAIL);
}

static void render(uint64_t frame, long tsec) {
  fill_rect(0, 0, fb_w, fb_h, C_BG);
  if (end_frame >= 0) {
    render_end((uint64_t)(frame - (uint64_t)end_frame));
    return;
  }
  render_rain(tsec);
  if (done_seen) render_stopped();
}

/* ---------------- device DRM setup (splash2 skeleton) ---------------- */
static uint32_t g_crtc_id, g_conn_id, g_pitch_px;
static struct drm_mode_modeinfo g_mode;
/* double-buffered dumb fbs: render into the back one, PAGE_FLIP to show.
 * If flips are refused, we fall back to re-SETCRTC latching. */
static uint32_t g_fb[2];
static uint32_t *g_map[2];
static int g_cur;                 /* currently displayed buffer */

/* Prepare everything and mmap both dumb fbs. Split from the mode set so the
 * FIRST frame can be painted before SETCRTC: on this panel the scanout
 * picks up the fb contents as of the mode set (splash2 painted-then-set
 * and showed; bootcard's set-then-paint left a black screen with the
 * backlight on — observed 2026-08-28). Call drm_modeset() after render(). */
static int drm_prepare(void) {
  int fd = open("/dev/dri/card0", O_RDWR | O_CLOEXEC);
  if (fd < 0) { kmsg("bootcard: no card0\n"); return -1; }
  ioctl(fd, DRM_IOCTL_SET_MASTER);

  struct drm_mode_card_res res;
  memset(&res, 0, sizeof res);
  if (ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res)) { kmsg("bootcard: GETRESOURCES fail\n"); close(fd); return -1; }
  uint32_t crtcs[16], conns[16];
  if (res.count_crtcs > 16) res.count_crtcs = 16;
  if (res.count_connectors > 16) res.count_connectors = 16;
  res.crtc_id_ptr = (uint64_t)(uintptr_t)crtcs;
  res.connector_id_ptr = (uint64_t)(uintptr_t)conns;
  /* msm_drm (4.19 sde) rejects a second GETRESOURCES if count_fbs/encoders
   * are nonzero but their pointers are null - zero those counts too. */
  res.count_fbs = 0;
  res.count_encoders = 0;
  if (ioctl(fd, DRM_IOCTL_MODE_GETRESOURCES, &res)) { close(fd); return -1; }

  uint32_t conn_id = 0, enc_id = 0;
  struct drm_mode_modeinfo mode;
  memset(&mode, 0, sizeof mode);
  struct drm_mode_modeinfo modes[8];
  uint64_t encs[8];
  for (uint32_t i = 0; i < res.count_connectors; i++) {
    struct drm_mode_get_connector gc;
    memset(&gc, 0, sizeof gc);
    gc.connector_id = conns[i];
    gc.encoders_ptr = (uint64_t)(uintptr_t)encs;
    gc.count_encoders = 8;
    gc.modes_ptr = (uint64_t)(uintptr_t)modes;
    gc.count_modes = 8;
    if (ioctl(fd, DRM_IOCTL_MODE_GETCONNECTOR, &gc)) continue;
    if (gc.count_modes < 1) continue;
    if (gc.connector_type == 0) continue;
    /* Prefer a bound encoder (gc.encoder_id). But when the previous DRM
     * master exited — e.g. our own earlier run, or splash being replaced —
     * the driver releases the binding and encoder_id reads 0 even though
     * the connector still lists its compatible encoders (observed live:
     * "no enabled-path connector" from splash2 at t+2142 s). Fall back to
     * the first compatible encoder; SETCRTC rebinds it. */
    uint32_t e = gc.encoder_id ? gc.encoder_id
               : (gc.count_encoders > 0 ? (uint32_t)encs[0] : 0);
    if (!e) continue;
    if (!conn_id || gc.connector_type == 16) {
      conn_id = conns[i];
      enc_id = e;
      mode = modes[0];
      if (gc.connector_type == 16) break;
    }
  }
  if (!conn_id) { kmsgf("bootcard: no usable connector\n"); close(fd); return -1; }

  uint32_t crtc_id = 0;
  struct drm_mode_get_encoder ge;
  memset(&ge, 0, sizeof ge);
  ge.encoder_id = enc_id;
  if (!ioctl(fd, DRM_IOCTL_MODE_GETENCODER, &ge)) {
    if (ge.crtc_id) crtc_id = ge.crtc_id;
    else for (uint32_t c = 0; c < res.count_crtcs; c++)
      if (ge.possible_crtcs & (1u << c)) { crtc_id = crtcs[c]; break; }
  }
  if (!crtc_id) { kmsgf("bootcard: no crtc for enc %u\n", enc_id); close(fd); return -1; }
  g_crtc_id = crtc_id;
  g_mode = mode;

  for (int b = 0; b < 2; b++) {
    struct drm_mode_create_dumb dumb;
    memset(&dumb, 0, sizeof dumb);
    dumb.width = mode.hdisplay; dumb.height = mode.vdisplay; dumb.bpp = 32;
    if (ioctl(fd, DRM_IOCTL_MODE_CREATE_DUMB, &dumb)) { kmsgf("bootcard: DUMB fail %d\n", errno); close(fd); return -1; }
    struct drm_mode_fb_cmd2 fb2;
    memset(&fb2, 0, sizeof fb2);
    fb2.width = mode.hdisplay; fb2.height = mode.vdisplay;
    fb2.pixel_format = DRM_FORMAT_XRGB8888;
    fb2.handles[0] = dumb.handle; fb2.pitches[0] = dumb.pitch;
    if (ioctl(fd, DRM_IOCTL_MODE_ADDFB2, &fb2)) { kmsgf("bootcard: ADDFB2 fail %d\n", errno); close(fd); return -1; }
    struct drm_mode_map_dumb map;
    memset(&map, 0, sizeof map);
    map.handle = dumb.handle;
    if (ioctl(fd, DRM_IOCTL_MODE_MAP_DUMB, &map)) { kmsgf("bootcard: MAP fail %d\n", errno); close(fd); return -1; }
    uint32_t *m = mmap(NULL, dumb.size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, map.offset);
    if (m == MAP_FAILED) { kmsgf("bootcard: mmap fail %d\n", errno); close(fd); return -1; }
    g_fb[b] = fb2.fb_id;
    g_map[b] = m;
    g_pitch_px = dumb.pitch / 4;
  }
  fb_w = mode.hdisplay; fb_h = mode.vdisplay;
  g_conn_id = conn_id;
  return fd;   /* keep fd open for the lifetime — it holds DRM master */
}

static int drm_modeset(int fd, uint32_t fb_id) {
  uint32_t conn_list[1] = { g_conn_id };
  struct drm_mode_crtc sc;
  memset(&sc, 0, sizeof sc);
  sc.crtc_id = g_crtc_id; sc.fb_id = fb_id; sc.x = 0; sc.y = 0;
  sc.mode = g_mode; sc.mode_valid = 1;
  sc.set_connectors_ptr = (uint64_t)(uintptr_t)conn_list;
  sc.count_connectors = 1;
  int rc = ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &sc);
  if (rc) {
    sc.set_connectors_ptr = 0; sc.count_connectors = 0;
    rc = ioctl(fd, DRM_IOCTL_MODE_SETCRTC, &sc);
  }
  if (rc) {
    kmsgf("bootcard: SETCRTC fail errno=%d\n", errno);
    return -1;
  }
  return 0;
}

/* ---------------- PPM (host verification) ---------------- */
static int ppm_write(const char *path) {
  FILE *f = fopen(path, "wb");
  if (!f) return -1;
  fprintf(f, "P6\n%u %u\n255\n", fb_w, fb_h);
  for (uint32_t y = 0; y < fb_h; y++)
    for (uint32_t x = 0; x < fb_w; x++) {
      uint32_t p = pix[y * pitch_px + x];
      unsigned char rgb[3] = { (p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff };
      fwrite(rgb, 1, 3, f);
    }
  fclose(f);
  return 0;
}

static void self_state(void) {
  st_status[0] = ST_OK;                       /* kernel */
  char rel[64] = "";
  int fd = open("/proc/sys/kernel/osrelease", O_RDONLY);
  if (fd >= 0) {
    int n = read(fd, rel, sizeof rel - 1);
    close(fd);
    if (n > 0) { rel[n] = 0; rel[strcspn(rel, "\n")] = 0; }
  }
  snprintf(st_detail[0], sizeof st_detail[0], "%s", rel);
  st_status[1] = ST_OK;                       /* rootfs */
  snprintf(st_detail[1], sizeof st_detail[1], "ext4 / userdata");
}

/* boot.state for the host preview modes; the demo lines mirror a mid-boot
 * device (wifi still running), which is when the rain is on screen. */
static void load_state_or_demo(const char *statepath) {
  if (statepath && access(statepath, R_OK) == 0) {
    read_state(statepath);
    return;
  }
  const char *demo[] = { "kernel ok 5.4.61-android13", "rootfs ok ext4",
    "display ok 1080x2340", "touch ok", "battery ok 87%",
    "modem ok", "wlan ok wlan0", "wifi run" };
  unlink("/tmp/bootcard.demo.state");
  for (unsigned i = 0; i < sizeof demo / sizeof demo[0]; i++) {
    FILE *f = fopen("/tmp/bootcard.demo.state", "a");
    fprintf(f, "%s\n", demo[i]);
    fclose(f);
  }
  read_state("/tmp/bootcard.demo.state");
}

int main(int argc, char **argv) {
  font_init();

  if (argc > 1 && !strcmp(argv[1], "--ppm")) {
    const char *out = argc > 2 ? argv[2] : "/tmp/bootcard.ppm";
    load_state_or_demo(argc > 3 ? argv[3] : NULL);
    self_state();
    fb_w = 1080; fb_h = 2340; pitch_px = fb_w;
    pix = malloc((size_t)fb_w * fb_h * 4);
    rain_init(0);                       /* unseeded: deterministic host frame */
    if (boot_success()) end_frame = 0;
    uint64_t fr = argc > 4 ? (uint64_t)strtoul(argv[4], NULL, 0)
                           : (end_frame >= 0 ? 50 : 3);
    /* advance the rain to tick fr so host previews match the device loop
     * (render() itself performs the fr-th advance) */
    if (end_frame < 0)
      for (uint64_t k = 1; k < fr; k++) render_rain(84);
    render(fr, 84);
    return ppm_write(out) ? 1 : 0;
  }

  if (argc > 1 && !strcmp(argv[1], "--ppm-seq")) {
    /* host-only: dump the exact frame run the device loop displays — one
     * PPM per tick at the panel pace (~16 fps) — for screen-faithful
     * recordings. args: <dir> <nframes> [end_at] [statefile]; frames past
     * end_at play the END chain (0 = rain only). */
    const char *dir = argc > 2 ? argv[2] : "/tmp/bcseq";
    uint64_t n = argc > 3 ? (uint64_t)strtoul(argv[3], NULL, 0) : 64;
    uint64_t end_at = argc > 4 ? (uint64_t)strtoul(argv[4], NULL, 0) : 0;
    load_state_or_demo(argc > 5 ? argv[5] : NULL);
    self_state();
    fb_w = 1080; fb_h = 2340; pitch_px = fb_w;
    pix = malloc((size_t)fb_w * fb_h * 4);
    rain_init(0);
    for (uint64_t k = 0; k < n; k++) {
      if (end_at && end_frame < 0 && k >= end_at)
        end_frame = (long long)k;   /* done just landed */
      char path[256];
      snprintf(path, sizeof path, "%s/f%04llu.ppm", dir,
               (unsigned long long)k);
      render(k, (long)(k / 16));    /* 16 fps wall clock for the footer */
      if (ppm_write(path)) return 1;
    }
    return 0;
  }

  const char *statepath = argc > 1 ? argv[1] : "/run/boot.state";
  struct timespec t0;
  clock_gettime(CLOCK_MONOTONIC, &t0);
  self_state();

  int fd = -1;
  for (int tries = 0; tries < 300; tries++) {
    fd = drm_prepare();
    if (fd >= 0) break;
    sleep(2);      /* msm_drm + panel registration take ~60 s after rcS */
  }
  if (fd < 0) {
    kmsg("bootcard: DRM never came up; staying alive to log state\n");
    fprintf(stderr, "bootcard: no panel; logging state only\n");
    for (;;) {
      if (read_state(statepath)) {
        for (int i = 0; i < NKEYS; i++)
          fprintf(stderr, "state: %s %d %s\n", KEYS[i], st_status[i], st_detail[i]);
      }
      sleep(1);
    }
  }
  st_status[2] = ST_OK;                       /* display: we are about to prove it */
  snprintf(st_detail[2], sizeof st_detail[2], "%ux%u DSI", fb_w, fb_h);
  pitch_px = g_pitch_px;

  /* FIRST frame before the mode set — see the drm_prepare comment: the
   * scanout snapshot happens at SETCRTC. */
  pix = g_map[0];
  render(0, 0);
  if (drm_modeset(fd, g_fb[0])) {
    kmsg("bootcard: modeset failed; logging state only\n");
    for (;;) {
      if (read_state(statepath))
        fprintf(stderr, "bootcard: state changed (no panel)\n");
      sleep(1);
    }
  }
  kmsgf("bootcard: panel up %ux%u conn=%u\n", fb_w, fb_h, g_conn_id);
  rain_init(1);                 /* seeded: rain differs boot to boot */
  g_cur = 0;

  /* Present path: try PAGE_FLIP once; this msm_drm 4.19 refuses it
   * (atomic-only driver, errno 2 — same as aginx-term's drm.rs), so the
   * working path is drm.rs's: re-SETCRTC relatch on EVERY frame. */
  int flip_ok = 1;
  uint64_t frame = 0;
  for (;;) {
    int changed = read_state(statepath);
    if (end_frame < 0 && boot_success()) {
      end_frame = (long long)frame;
      kmsg("bootcard: end sequence\n");
    }
    if (changed)
      for (int i = 0; i < NKEYS; i++)
        if (st_status[i] != ST_PEND || st_detail[i][0])
          fprintf(stderr, "bootcard: %s %s %s\n", KEYS[i],
                  st_status[i] == ST_OK ? "ok" : st_status[i] == ST_FAIL ? "fail"
                  : st_status[i] == ST_RUN ? "run" : "-",
                  st_detail[i]);
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    long tsec = now.tv_sec - t0.tv_sec;
    int next = 1 - g_cur;
    pix = g_map[next];
    render(frame, tsec);
    if (flip_ok) {
      struct drm_mode_crtc_page_flip pf;
      memset(&pf, 0, sizeof pf);
      pf.fb_id = g_fb[next];
      pf.crtc_id = g_crtc_id;
      pf.flags = 1;   /* DRM_MODE_PAGE_FLIP_EVENT */
      if (ioctl(fd, DRM_IOCTL_MODE_PAGE_FLIP, &pf) == 0) {
        g_cur = next;
        /* consume the flip-complete event (latch confirmed at vblank);
         * bounded wait — a driver that never delivers events must not
         * hang the bootcard */
        struct pollfd p = { fd, POLLIN, 0 };
        if (poll(&p, 1, 200) > 0) {
          char evbuf[64];
          ssize_t r;
          do { r = read(fd, evbuf, sizeof evbuf); } while (r > 0);
        }
      } else {
        flip_ok = 0;
        kmsgf("bootcard: PAGE_FLIP refused (%d) — relatch per frame\n", errno);
      }
    }
    if (!flip_ok) {
      /* msm_drm 4.19 is atomic-only and refuses legacy flips (errno 2,
       * 2026-09-07); drm.rs's proven path is re-SETCRTC relatch on EVERY
       * present (aginx-term runs 14 fps that way). Relatching every Nth
       * frame is the stutter the boot rain showed. */
      drm_modeset(fd, g_fb[next]);
      g_cur = next;
    }
    frame++;
    usleep(40000);
  }
}
