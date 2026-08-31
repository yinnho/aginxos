/* media-topo — media-controller topology dump (M19).
 *
 * Lists every entity of each /dev/mediaN device (name, type, pads,
 * devnode major:minor) and the pad-to-pad links. Our rootfs has no
 * media-ctl; this is the freestanding equivalent — same ioctl-only
 * style as snd-mixer. Structs embedded from uapi linux/media.h (4.19,
 * stable ABI) so no header dependency is required.
 *
 * Usage: media-topo [/dev/media0 ...]
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
#include <stdint.h>

struct media_device_info {
	char driver[16];
	char model[32];
	char serial[40];
	char bus_info[32];
	uint32_t media_version;
	uint32_t hw_revision;
	uint32_t driver_version;
	uint32_t reserved[31];
};

struct media_entity_desc {
	uint32_t id;
	char name[32];
	uint32_t type;
	uint32_t revision;
	uint32_t flags;
	uint32_t group_id;
	uint16_t pads;
	uint16_t links;
	uint32_t reserved[4];
	union {
		struct { uint32_t major, minor; } v4l;
		struct { uint32_t major, minor; } fb;
		struct { uint32_t card, device; } alsa;
		struct { uint32_t major, minor; } dvb;
		uint8_t raw[184];
	} dev;
};

struct media_pad_desc {
	uint32_t entity;
	uint16_t index;
	uint32_t flags;
	uint32_t reserved[2];
};

struct media_link_desc {
	struct media_pad_desc source;
	struct media_pad_desc sink;
	uint32_t flags;
	uint32_t reserved[2];
};

struct media_links_enum {
	uint32_t entity;
	struct media_pad_desc *pads;
	struct media_link_desc *links;
	uint32_t reserved[4];
};

#define MEDIA_IOC_DEVICE_INFO   _IOWR('|', 0x00, struct media_device_info)
#define MEDIA_IOC_ENUM_ENTITIES _IOWR('|', 0x01, struct media_entity_desc)
#define MEDIA_IOC_ENUM_LINKS    _IOWR('|', 0x02, struct media_links_enum)
#define MEDIA_ENT_ID_FLAG_NEXT  ((uint32_t)1 << 31)

#define MEDIA_PAD_FL_SINK   0x0001
#define MEDIA_PAD_FL_SOURCE 0x0002

static void describe_type(uint32_t type, char *out, size_t n)
{
	if (type & 0x80000000u) { /* old MEDIA_ENT_T_DEVNODE_* */
		snprintf(out, n, "devnode/%#x", type);
	} else if (type & 0x00020000u) {
		const char *s = "subdev";
		switch (type) {
		case 0x00010001: s = "CAM_SENSOR"; break;
		case 0x00010002: s = "FLASH"; break;
		case 0x00010003: s = "LENS"; break;
		case 0x00010004: s = "ATV_DECODER"; break;
		case 0x00010005: s = "TUNER"; break;
		}
		snprintf(out, n, "%s/%#x", s, type);
	} else {
		snprintf(out, n, "interface/%#x", type);
	}
}

int main(int argc, char **argv)
{
	char *def[2] = { NULL, NULL };
	if (argc < 2) {
		static char n0[] = "/dev/media0";
		def[0] = n0;
		argv = def;
		argc = 2;
	}
	for (int i = 1; i < argc; i++) {
		int fd = open(argv[i], O_RDONLY);
		if (fd < 0) { perror(argv[i]); continue; }
		struct media_device_info info;
		memset(&info, 0, sizeof info);
		if (ioctl(fd, MEDIA_IOC_DEVICE_INFO, &info) < 0) {
			perror("DEVICE_INFO"); close(fd); continue;
		}
		printf("%s: driver %s model %s ver %u.%u.%u\n", argv[i],
		       info.driver, info.model,
		       info.media_version >> 16, (info.media_version >> 8) & 0xff,
		       info.media_version & 0xff);

		uint32_t id = 0;
		struct ent { uint32_t id; char name[32]; uint16_t pads, links; };
		struct ent ents[256]; int nent = 0;
		for (;;) {
			struct media_entity_desc e;
			memset(&e, 0, sizeof e);
			e.id = id | MEDIA_ENT_ID_FLAG_NEXT;
			if (ioctl(fd, MEDIA_IOC_ENUM_ENTITIES, &e) < 0)
				break;
			id = e.id;
			if (nent < 256) {
				ents[nent].id = id;
				memcpy(ents[nent].name, e.name, 32);
				ents[nent].pads = e.pads;
				ents[nent].links = e.links;
				nent++;
			}
			char t[32];
			describe_type(e.type, t, sizeof t);
			char dev[48] = "";
			if (e.dev.v4l.major || e.dev.v4l.minor)
				snprintf(dev, sizeof dev, " dev %u:%u",
					 e.dev.v4l.major, e.dev.v4l.minor);
			printf("  ent %3u %-28s %-16s pads %u links %u%s\n",
			       id, e.name, t, e.pads, e.links, dev);
		}
		/* links: the kernel fills exactly entity->links entries */
		for (int k = 0; k < nent; k++) {
			if (ents[k].links == 0) continue;
			struct media_pad_desc pads[32];
			struct media_link_desc links[128];
			if (ents[k].links > 128) continue;
			struct media_links_enum le;
			memset(&le, 0, sizeof le);
			le.entity = ents[k].id;
			le.pads = pads;
			le.links = links;
			if (ioctl(fd, MEDIA_IOC_ENUM_LINKS, &le) < 0)
				continue;
			for (int j = 0; j < ents[k].links; j++) {
				const char *sf = (links[j].source.flags &
						  MEDIA_PAD_FL_SOURCE) ? "src" : "pad";
				printf("  link ent%u:%u (%s) -> ent%u:%u%s\n",
				       links[j].source.entity, links[j].source.index, sf,
				       links[j].sink.entity, links[j].sink.index,
				       (links[j].flags & 1) ? " ENABLED" : "");
			}
		}
		close(fd);
	}
	return 0;
}
