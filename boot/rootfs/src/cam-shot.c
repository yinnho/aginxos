// cam-shot (M19 v0) — Qualcomm cam_req_mgr userspace, first tool: sensor probe.
//
// What it does: for each cam-sensor subdev (found via media2 entity type
// CAM_SENSOR_DEVICE_TYPE), issue CAM_SENSOR_PROBE_CMD with a hand-built
// probe packet: cam_cmd_i2c_info + cam_cmd_probe (cmd buf 0) and a
// power-sequence blob (cmd buf 1). The kernel powers the sensor per the
// blob, reads the chip-id register over CCI, and powers down. We send
// expected_id=0xFFFF/data_mask=0 on purpose: the compare always misses,
// CAM_WARN "read id: 0x.. expected id: 0xffff" prints the REAL chip id to
// kmsg, and the sensor is left unprobed (retryable). If a slave address
// NACKs the read returns 0 — we report absence and try the next address.
//
// Packet protocol reference: techpack/camera uapi cam_defs.h / cam_sensor.h
// / cam_req_mgr.h (LineageOS redbull, lineage-22.1) — structs mirrored
// below, packed, LE like the kernel.
//
// Power sequences and rails come from the device DT (dumped 2026-08-31,
// see HARDWARE.md M19): sensor@0 vio/vana/vdig 2.85/vaf + reset tlmm23 +
// mclk tlmm13; sensor@1 vio/vana/vdig1.1 + reset tlmm25 + mclk tlmm14;
// sensor@2 vio + custom_gpio1 (PM8150L gpio2) + reset tlmm21 + mclk tlmm15.
// Rail voltages: config_val=0 means "keep DT min/max" (VALIDATE_VOLTAGE
// rejects 0, kernel keeps DT values) — msm_camera_fill_vreg_params maps
// seq_type -> rail index by name, missing rail -> INVALID_VREG -> skipped.

#include <fcntl.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

/* ---- cam_defs.h ---- */
struct cam_control {
    uint32_t op_code;
    uint32_t size;
    uint32_t handle_type;
    uint32_t reserved;
    uint64_t handle;
};
#define VIDIOC_CAM_CONTROL 0xC01856C0 /* _IOWR('V', 192, 24) */
#define CAM_QUERY_CAP   0x101
#define CAM_HANDLE_USER_POINTER 1
#define CAM_HANDLE_MEM_HANDLE   2

struct cam_packet_header {
    uint32_t op_code, size;
    uint64_t request_id;
    uint32_t flags, padding;
};
struct cam_cmd_buf_desc {
    int32_t  mem_handle;
    uint32_t offset, size, length, type, meta_data;
};
struct cam_packet {
    struct cam_packet_header header;
    uint32_t cmd_buf_offset, num_cmd_buf;
    uint32_t io_configs_offset, num_io_configs;
    uint32_t patch_offset, num_patches;
    uint32_t kmd_cmd_buf_index, kmd_cmd_buf_offset;
    uint64_t payload[1];
};

/* ---- cam_req_mgr.h (video3 opcodes + mem mgr) ---- */
#define CAM_REQ_MGR_CREATE_DEV_NODES 0x10A
#define CAM_REQ_MGR_ALLOC_BUF   0x112
#define CAM_REQ_MGR_MAP_BUF     0x113
#define CAM_REQ_MGR_RELEASE_BUF 0x114
#define CAM_REQ_MGR_CACHE_OPS   0x115
#define CAM_MEM_FLAG_KMD_ACCESS  (1 << 3)
#define CAM_MEM_FLAG_CMD_BUF_TYPE (1 << 6)
struct cam_mem_mgr_alloc_cmd {
    uint64_t len, align;
    int32_t  mmu_hdls[16];
    uint32_t num_hdl, flags;
    struct { uint32_t buf_handle; int32_t fd; uint64_t vaddr; } out;
};
struct cam_mem_mgr_release_cmd { int32_t buf_handle; uint32_t reserved; };

/* ---- cam_sensor.h ---- */
#define CAM_SENSOR_PROBE_CMD (0x109 + 1)
struct cam_cmd_i2c_info { uint32_t slave_addr; uint8_t i2c_freq_mode,
    cmd_type; uint16_t reserved; } __attribute__((packed));
struct cam_cmd_probe {
    uint8_t data_type, addr_type, op_code, cmd_type;
    uint32_t reg_addr, expected_data, data_mask;
    uint16_t camera_id; uint8_t fw_update_flag; uint16_t reserved;
} __attribute__((packed));
struct cam_power_settings {
    uint16_t power_seq_type, reserved;
    uint32_t config_val_low, config_val_high;
} __attribute__((packed));
struct cam_cmd_power {
    uint32_t count; uint8_t reserved, cmd_type; uint16_t more_reserved;
    struct cam_power_settings power_settings[1];
} __attribute__((packed));
struct cam_cmd_unconditional_wait {
    int16_t delay, reserved; uint8_t op_code, cmd_type; uint16_t reserved1;
} __attribute__((packed));
struct cam_sensor_query_cap {
    uint32_t slot_info, secure_camera, pos_pitch, pos_roll, pos_yaw,
        actuator_slot_id, eeprom_slot_id, ois_slot_id, flash_slot_id,
        csiphy_slot_id;
} __attribute__((packed));

/* enums from cam_sensor_cmn_header.h */
enum { SENSOR_MCLK = 0, SENSOR_VANA, SENSOR_VDIG, SENSOR_VIO, SENSOR_VAF,
       SENSOR_VAF_PWDM, SENSOR_CUSTOM_REG1, SENSOR_CUSTOM_REG2,
       SENSOR_RESET = 8, SENSOR_STANDBY, SENSOR_CUSTOM_GPIO1 = 10 };
#define CMD_PWR_UP   2
#define CMD_PWR_DOWN 3
#define CMD_I2C_INFO 4
#define CMD_PROBE    1
#define CMD_WAIT     9
#define WAIT_SW_UCND 3
#define I2C_TYPE_WORD 2
#define I2C_FREQ_FAST 1

/* media controller bits (same struct as media-topo, 4.19 uapi) */
struct media_entity_desc {
    uint32_t id;
    char name[32];
    uint32_t type, revision, flags, group_id;
    uint16_t pads, links;
    uint32_t reserved[4];
    union {
        struct { uint32_t major, minor; } v4l;
        uint8_t raw[184];
    } dev;
};
#define MEDIA_IOC_ENUM_ENTITIES _IOWR('|', 0x01, struct media_entity_desc)
#define CAM_SENSOR_DEVICE_TYPE 0x10001
#define MEDIA_ENT_ID_FLAG_NEXT ((uint32_t)1 << 31)

#define MAX_SUBDEV 8

struct slot_cfg {
    const char *name;
    uint32_t addr;   /* 8-bit CCI slave address for the real probe */
    /* power-up steps: seq_type, config_val, delay_ms */
    struct { uint8_t seq; uint32_t cfg; uint16_t delay; } up[8];
    int n_up;
    /* power-down steps (applied in given order) */
    struct { uint8_t seq; uint32_t cfg; uint16_t delay; } down[8];
    int n_down;
};

static struct slot_cfg slots[3] = {
    /* slot 0 rails (DT phandles resolved): cam_vio=slg51000 ldo7 1.8V,
     * cam_vana=ldo3, cam_vdig=ldo1, cam_v_custom1=ldo4, cam_v_custom2=ldo6,
     * cam_vaf=gpio-regulator@0 "camera_ldo" (pm8150l gpio8 camera_rear_vcm_en).
     * The kernel power-up executor (cam_sensor_core_power_up) only enables
     * rails that appear in the seq — no fallback — so omitting VIO leaves
     * the sensor's I2C/DOVDD unpowered and the chip-ID read NACKs.
     * Address: IMX3xx latches one of two slave addrs from the INCK/XCLR
     * power-on timing; our seq (MCLK before XCLR) latches 0x20 — observed
     * via full-bus sweep 2026-09-01, chip id 0x363 read at 0x20. */
    [0] = { "rear-main", .addr = 0x20, .up = {
        { SENSOR_VIO,         0, 1 }, { SENSOR_VANA, 0, 1 },
        { SENSOR_VAF,         0, 0 }, { SENSOR_VDIG, 0, 1 },
        { SENSOR_CUSTOM_REG1, 0, 1 }, { SENSOR_CUSTOM_REG2, 0, 1 },
        { SENSOR_MCLK,        0, 1 }, { SENSOR_RESET, 1, 5 } },
        .n_up = 8, .down = {
        { SENSOR_MCLK,        0, 1 }, { SENSOR_RESET, 0, 1 },
        { SENSOR_CUSTOM_REG2, 0, 1 }, { SENSOR_CUSTOM_REG1, 0, 1 },
        { SENSOR_VDIG,        0, 1 }, { SENSOR_VAF,  0, 1 },
        { SENSOR_VANA,        0, 1 }, { SENSOR_VIO,  0, 1 } }, .n_down = 8 },
    [1] = { "rear-uw", .addr = 0x34, .up = {
        { SENSOR_VIO,   0, 1 }, { SENSOR_VANA,  0, 1 },
        { SENSOR_VDIG,  0, 1 }, { SENSOR_RESET, 1, 8 },
        { SENSOR_MCLK,  0, 1 } }, .n_up = 5, .down = {
        { SENSOR_MCLK,  0, 1 }, { SENSOR_RESET, 0, 5 },
        { SENSOR_VDIG,  0, 1 }, { SENSOR_VANA,  0, 1 },
        { SENSOR_VIO,   0, 1 } }, .n_down = 5 },
    [2] = { "front", .addr = 0x34, .up = {
        { SENSOR_VIO,          0, 1 },
        { SENSOR_CUSTOM_GPIO1, 1, 5 },   /* PM8150L gpio2 */
        { SENSOR_RESET,        1, 8 },
        { SENSOR_MCLK,         0, 1 } }, .n_up = 4, .down = {
        { SENSOR_MCLK,         0, 1 },
        { SENSOR_RESET,        0, 5 },
        { SENSOR_CUSTOM_GPIO1, 0, 1 },
        { SENSOR_VIO,          0, 1 } }, .n_down = 4 },
};

static const uint32_t try_addrs[] = { 0x34, 0x20, 0x10, 0x6E, 0x6C, 0x36 };

/* ---- helpers ---- */
/* NB: video3 (cam_req_mgr) checks size == sizeof(payload struct), so the
 * size field must carry the payload size, not sizeof(cam_control). */
static int cam_ioctl(int fd, uint32_t op, void *arg, uint32_t htype,
                     uint32_t size)
{
    struct cam_control ctl = { .op_code = op, .size = size,
        .handle_type = htype, .reserved = 0,
        .handle = (uint64_t)(uintptr_t)arg };
    return ioctl(fd, VIDIOC_CAM_CONTROL, &ctl);
}

static int alloc_buf(int video_fd, uint64_t len, uint64_t align,
                     struct cam_mem_mgr_alloc_cmd *out)
{
    memset(out, 0, sizeof(*out));
    out->len = len;
    out->align = align;
    out->num_hdl = 0;
    out->flags = CAM_MEM_FLAG_KMD_ACCESS | CAM_MEM_FLAG_CMD_BUF_TYPE;
    if (cam_ioctl(video_fd, CAM_REQ_MGR_ALLOC_BUF, out,
                  CAM_HANDLE_USER_POINTER, sizeof(*out)) < 0) {
        fprintf(stderr, "alloc_buf(%llu) failed: %s\n",
            (unsigned long long)len, strerror(errno));
        return -1;
    }
    return 0;
}

static void release_buf(int video_fd, uint32_t hdl)
{
    struct cam_mem_mgr_release_cmd rel = { .buf_handle = (int32_t)hdl };
    cam_ioctl(video_fd, CAM_REQ_MGR_RELEASE_BUF, &rel,
              CAM_HANDLE_USER_POINTER, sizeof(rel));
}

static void *map_fd(int fd, size_t len)
{
    void *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (p == MAP_FAILED)
        fprintf(stderr, "mmap failed: %s\n", strerror(errno));
    return p;
}

/* kmsg tail: print CAM_SENSOR lines newer than the given monotonic us. */
static double kmsg_drain(int fd, double since_us)
{
    char buf[512];
    double max = since_us;
    while (1) {
        ssize_t n = read(fd, buf, sizeof(buf) - 1);
        if (n <= 0)
            break;
        buf[n] = 0;
        double ts;
        if (sscanf(buf, "%*d,%*llu,%lf;%*s", &ts) != 1) {
            /* fall back: whole line after ';' */
            char *semi = strchr(buf, ';');
            if (!semi)
                continue;
            ts = max;
        }
        if (ts > max)
            max = ts;
        if (ts < since_us)
            continue;
        char *semi = strchr(buf, ';');
        if (semi && (strstr(semi, "CAM-SENSOR") || strstr(semi, "cam_req") ||
                     strstr(semi, "CAM-UTIL") || strstr(semi, "CAM-CCI") ||
                     strstr(semi, "slg51000")))
            printf("    kmsg: %s", semi + 1);
    }
    return max;
}

/* classify camera kmsg lines since ts (quiet): ACK / NACK / bus wedge */
#define KMSG_QUIET 0
#define KMSG_NACK  1
#define KMSG_HIT   2
#define KMSG_WEDGE 3
static int kmsg_classify(int fd, double since_us, uint32_t *hit_id,
                         double *out_max)
{
    char buf[512];
    int st = KMSG_QUIET;
    double max = since_us;
    if (hit_id)
        *hit_id = 0;
    while (1) {
        ssize_t n = read(fd, buf, sizeof(buf) - 1);
        if (n <= 0)
            break;
        buf[n] = 0;
        double ts;
        if (sscanf(buf, "%*d,%*llu,%lf;%*s", &ts) != 1) {
            char *semi = strchr(buf, ';');
            if (!semi)
                continue;
            ts = max;
        }
        if (ts > max)
            max = ts;
        if (ts < since_us)
            continue;
        char *semi = strchr(buf, ';');
        if (!semi || !strstr(semi, "CAM-"))
            continue;   /* only camera-driver lines (adbd spam says "timeout") */
        char *m = strstr(semi, "read id: 0x");
        if (m) {
            uint32_t id = (uint32_t)strtoul(m + 11, 0, 16);
            if (id) {
                st = KMSG_HIT;
                if (hit_id)
                    *hit_id = id;
            } else if (st < KMSG_NACK) {
                st = KMSG_NACK;
            }
        }
        if (strstr(semi, "-110") || strstr(semi, "timeout")) {
            printf("    wedge-match line: %s", semi + 1);
            st = KMSG_WEDGE;
        }
    }
    if (out_max)
        *out_max = max;
    return st;
}

/* ---- packet build ---- */
struct bufs {
    struct cam_mem_mgr_alloc_cmd pkt, c0, c1;
    void *p_pkt, *p_c0, *p_c1;
    int pkt_fd, c0_fd, c1_fd;
};

static void bufs_free(int video_fd, struct bufs *b)
{
    if (b->p_pkt) munmap(b->p_pkt, 4096);
    if (b->p_c0)  munmap(b->p_c0, 256);
    if (b->p_c1)  munmap(b->p_c1, 1024);
    if (b->pkt.out.buf_handle) release_buf(video_fd, b->pkt.out.buf_handle);
    if (b->c0.out.buf_handle)  release_buf(video_fd, b->c0.out.buf_handle);
    if (b->c1.out.buf_handle)  release_buf(video_fd, b->c1.out.buf_handle);
    if (b->pkt_fd > 0) close(b->pkt_fd);
    if (b->c0_fd > 0)  close(b->c0_fd);
    if (b->c1_fd > 0)  close(b->c1_fd);
}

static int probe_once(int video_fd, int sd_fd, int slot, uint32_t slave,
                      uint32_t reg, uint32_t expected)
{
    struct bufs b;
    memset(&b, 0, sizeof(b));
    int rc = -1;

    if (alloc_buf(video_fd, 4096, 4096, &b.pkt) < 0) goto out;
    if (alloc_buf(video_fd, 256, 8, &b.c0) < 0) goto out;
    if (alloc_buf(video_fd, 1024, 8, &b.c1) < 0) goto out;
    b.pkt_fd = b.pkt.out.fd; b.c0_fd = b.c0.out.fd; b.c1_fd = b.c1.out.fd;
    b.p_pkt = map_fd(b.pkt_fd, 4096);
    b.p_c0 = map_fd(b.c0_fd, 256);
    b.p_c1 = map_fd(b.c1_fd, 1024);
    if (!b.p_pkt || !b.p_c0 || !b.p_c1) goto out;
    memset(b.p_pkt, 0, 4096); memset(b.p_c0, 0, 256); memset(b.p_c1, 0, 1024);

    /* cmd buf 0: i2c info + probe */
    struct cam_cmd_i2c_info *i2c = b.p_c0;
    i2c->slave_addr = slave;
    i2c->i2c_freq_mode = I2C_FREQ_FAST;
    i2c->cmd_type = CMD_I2C_INFO;
    struct cam_cmd_probe *pr = (void *)((char *)b.p_c0 + sizeof(*i2c));
    pr->data_type = I2C_TYPE_WORD;
    pr->addr_type = I2C_TYPE_WORD;
    pr->cmd_type = CMD_PROBE;
    pr->reg_addr = reg;
    /* expected=0xFFFF/mask=0: id_by_mask returns the full 16-bit id and the
     * compare always fails -> kmsg prints the real id (discovery mode).
     * A matching value here makes the probe genuinely succeed. */
    pr->expected_data = expected;
    pr->data_mask = 0;
    pr->camera_id = (uint16_t)slot;
    uint32_t c0_len = sizeof(*i2c) + sizeof(*pr);

    /* cmd buf 1: power blob — one PWR_UP(count=1)+WAIT per step, then downs */
    struct slot_cfg *sc = &slots[slot];
    uint8_t *q = b.p_c1;
    for (int i = 0; i < sc->n_up; i++) {
        struct cam_cmd_power *pw = (void *)q;
        pw->count = 1;
        pw->cmd_type = CMD_PWR_UP;
        pw->power_settings[0].power_seq_type = sc->up[i].seq;
        pw->power_settings[0].config_val_low = sc->up[i].cfg;
        q += sizeof(*pw);
        if (sc->up[i].delay) {
            struct cam_cmd_unconditional_wait *w = (void *)q;
            w->delay = sc->up[i].delay;
            w->op_code = WAIT_SW_UCND;
            w->cmd_type = CMD_WAIT;
            q += sizeof(*w);
        }
    }
    for (int i = 0; i < sc->n_down; i++) {
        struct cam_cmd_power *pw = (void *)q;
        pw->count = 1;
        pw->cmd_type = CMD_PWR_DOWN;
        pw->power_settings[0].power_seq_type = sc->down[i].seq;
        pw->power_settings[0].config_val_low = sc->down[i].cfg;
        q += sizeof(*pw);
        if (sc->down[i].delay) {
            struct cam_cmd_unconditional_wait *w = (void *)q;
            w->delay = sc->down[i].delay;
            w->op_code = WAIT_SW_UCND;
            w->cmd_type = CMD_WAIT;
            q += sizeof(*w);
        }
    }
    uint32_t c1_len = (uint32_t)(q - (uint8_t *)b.p_c1);

    /* packet: header + 2 cmd descs at payload (cmd_buf_offset = 0) */
    struct cam_packet *pkt = b.p_pkt;
    pkt->header.op_code = 0;
    pkt->header.size = sizeof(*pkt) + 2 * sizeof(struct cam_cmd_buf_desc);
    pkt->num_cmd_buf = 2;
    struct cam_cmd_buf_desc *desc = (void *)pkt->payload;
    desc[0].mem_handle = (int32_t)b.c0.out.buf_handle;
    desc[0].size = 256; desc[0].length = c0_len;
    desc[1].mem_handle = (int32_t)b.c1.out.buf_handle;
    desc[1].size = 1024; desc[1].length = c1_len;

    rc = cam_ioctl(sd_fd, CAM_SENSOR_PROBE_CMD,
                   (void *)(uintptr_t)b.pkt.out.buf_handle,
                   CAM_HANDLE_MEM_HANDLE, sizeof(struct cam_control));
out:
    bufs_free(video_fd, &b);
    return rc;
}

/* find sensor subdev nodes via media2 (or media3..) entity table */
static int find_sensor_nodes(int sd_slot_fd[MAX_SUBDEV], int sd_slot[MAX_SUBDEV])
{
    int n = 0;
    for (int mi = 0; mi < 8 && n < MAX_SUBDEV; mi++) {
        char path[32];
        snprintf(path, sizeof(path), "/dev/media%d", mi);
        int mfd = open(path, O_RDONLY);
        if (mfd < 0)
            continue;
        uint32_t id = 0;
        for (;;) {
            struct media_entity_desc ent;
            memset(&ent, 0, sizeof(ent));
            ent.id = id | MEDIA_ENT_ID_FLAG_NEXT;
            if (ioctl(mfd, MEDIA_IOC_ENUM_ENTITIES, &ent) < 0) {
                if (id == 0)
                    fprintf(stderr, "%s: enum failed: %s\n", path,
                        strerror(errno));
                break;
            }
            id = ent.id;
            if (ent.type == CAM_SENSOR_DEVICE_TYPE &&
                ent.dev.v4l.major != 0) {
                /* map major:minor -> /dev/v4l-subdevX via sysfs */
                char want[32];
                snprintf(want, sizeof(want), "%u:%u",
                    ent.dev.v4l.major, ent.dev.v4l.minor);
                for (int s = 0; s < 32; s++) {
                    char sf[96], dv[32];
                    snprintf(sf, sizeof(sf),
                        "/sys/class/video4linux/v4l-subdev%d/dev", s);
                    int df = open(sf, O_RDONLY);
                    if (df < 0)
                        continue;
                    ssize_t r = read(df, dv, sizeof(dv) - 1);
                    close(df);
                    if (r <= 0)
                        continue;
                    dv[r] = 0;
                    size_t wl = strlen(want);
                    if (strncmp(dv, want, wl) == 0 &&
                        (dv[wl] == '\n' || dv[wl] == 0)) {
                        char devp[32];
                        snprintf(devp, sizeof(devp),
                            "/dev/v4l-subdev%d", s);
                        int fd = open(devp, O_RDWR);
                        if (fd >= 0) {
                            struct cam_sensor_query_cap cap;
                            memset(&cap, 0, sizeof(cap));
                            if (cam_ioctl(fd, CAM_QUERY_CAP, &cap,
                                          CAM_HANDLE_USER_POINTER,
                                          sizeof(cap)) == 0) {
                                sd_slot_fd[n] = fd;
                                sd_slot[n] = (int)cap.slot_info;
                                printf("slot %d: %s (entity %s, csiphy %d, "
                                    "eeprom %d, actuator %d, ois %d)\n",
                                    cap.slot_info, devp, ent.name,
                                    cap.csiphy_slot_id, cap.eeprom_slot_id,
                                    cap.actuator_slot_id, cap.ois_slot_id);
                                n++;
                            } else {
                                printf("slot ?: %s querycap %s\n", devp,
                                    strerror(errno));
                            }
                        }
                        break;
                    }
                }
            }
        }
        close(mfd);
    }
    return n;
}

int main(int argc, char **argv)
{
    int only_slot = -1;
    int real = 0, sweep = 0;
    uint32_t reg = 0x0016;   /* Sony IMX3xx-family chip id register */
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--reg") == 0 && i + 1 < argc)
            reg = (uint32_t)strtoul(argv[++i], 0, 0);
        else if (strcmp(argv[i], "--real") == 0)
            real = 1;
        else if (strcmp(argv[i], "--sweep") == 0)
            sweep = 1;
        else
            only_slot = atoi(argv[i]);
    }

    int video_fd = open("/dev/video3", O_RDWR);
    if (video_fd < 0) {
        fprintf(stderr, "open /dev/video3: %s\n", strerror(errno));
        return 1;
    }

    int kmsg = open("/dev/kmsg", O_RDONLY | O_NONBLOCK);
    double kmark = 0;
    if (kmsg >= 0)
        kmark = kmsg_drain(kmsg, 0);   /* flush ring, remember newest */

    int sd_fd[MAX_SUBDEV], sd_slot[MAX_SUBDEV];
    int n = find_sensor_nodes(sd_fd, sd_slot);
    if (n == 0) {
        fprintf(stderr, "no sensor subdevs found\n");
        return 1;
    }

    /* chip ids observed on this device (HARDWARE.md 2026-08-31) */
    static const uint32_t slot_id[3] = { 0x363, 0x481, 0x355 };
    for (int i = 0; i < n; i++) {
        if (only_slot >= 0 && sd_slot[i] != only_slot) {
            close(sd_fd[i]);
            continue;
        }
        int slot = sd_slot[i];
        uint32_t expected = real ? slot_id[slot] : 0xFFFF;
        if (sweep) {
            /* walk every even 8-bit address on this slot's CCI bus to
             * answer "is anything alive on this bus at all?" — the rear
             * module bus (cci0/master0) carries sensor + eeprom +
             * actuator + ois, so any ACK proves bus+pins alive */
            printf("sweep slot %d (%s), all even addrs, reg 0x%04x\n",
                slot, slots[slot].name, reg);
            fflush(stdout);
            for (uint32_t a = 0x02; a <= 0xFE; a += 2) {
                double t0 = kmark;
                probe_once(video_fd, sd_fd[i], slot, a, reg, 0xFFFF);
                uint32_t id = 0;
                int st = kmsg_classify(kmsg, t0, &id, &kmark);
                if (st == KMSG_HIT)
                    printf("  0x%02x: ACK id=0x%04x\n", a, id);
                else if (st == KMSG_WEDGE) {
                    printf("  0x%02x: BUS WEDGE (timeout) — abort sweep\n", a);
                    fflush(stdout);
                    break;
                }
                if ((a & 0x1e) == 0) {
                    printf("  .. 0x%02x done\n", a);
                    fflush(stdout);
                }
            }
            close(sd_fd[i]);
            continue;
        }
        printf("probing slot %d (%s), id reg 0x%04x, expected 0x%04x\n",
            slot, slots[slot].name, reg, expected);
        if (real) {
            /* known sensor: single address, real expectation; the IMX363
             * (slg51000 rails, slow ramp) is cold-start flaky — retry */
            for (int attempt = 1; attempt <= 3; attempt++) {
                printf("  addr 0x%02x (try %d): ", slots[slot].addr,
                       attempt);
                fflush(stdout);
                double t0 = kmark;
                int rc = probe_once(video_fd, sd_fd[i], slot,
                                    slots[slot].addr, reg, expected);
                printf("rc=%d (%s)\n", rc,
                    rc == 0 ? "OK" : strerror(errno));
                kmark = kmsg_drain(kmsg, t0);
                if (rc == 0)
                    break;
                sleep(2);  /* let the module rails fully discharge */
            }
            close(sd_fd[i]);
            continue;
        }
        for (size_t a = 0; a < sizeof(try_addrs) / sizeof(try_addrs[0]);
             a++) {
            printf("  addr 0x%02x: ", try_addrs[a]);
            fflush(stdout);
            double t0 = kmark;
            int rc = probe_once(video_fd, sd_fd[i], slot,
                                try_addrs[a], reg, expected);
            if (rc == 0) {
                printf("probe rc=0 (unexpected match?)\n");
                kmark = kmsg_drain(kmsg, t0);
                break;
            }
            printf("rc=%d (%s)\n", rc, strerror(errno));
            kmark = kmsg_drain(kmsg, t0);
            /* the kmsg lines above carry the verdict: "read id: 0xNNN"
               = chip present at this addr; silence/cci errors = absent */
        }
        close(sd_fd[i]);
    }
    close(video_fd);
    if (kmsg >= 0)
        close(kmsg);
    return 0;
}
