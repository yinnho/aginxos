# AginxOS — Minimal Linux + Rust daemon
# Multi-arch: aarch64 + x86_64 | Custom kernel 6.1.12 + musl-static daemon

# ── Architecture selection ──
ARCH     ?= arm64
TARGET   := $(if $(filter arm64,$(ARCH)),aarch64,x86_64)-unknown-linux-musl
CROSS    := $(if $(filter arm64,$(ARCH)),aarch64,x86_64)-elf-
KARCH    := $(if $(filter arm64,$(ARCH)),arm64,x86)
KIMAGE   := $(if $(filter arm64,$(ARCH)),Image,bzImage)
KCONSOLE := $(if $(filter arm64,$(ARCH)),ttyAMA0,ttyS0)
QEMU_BIN := qemu-system-$(if $(filter arm64,$(ARCH)),aarch64,x86_64)
QMACHINE := $(if $(filter arm64,$(ARCH)),virt,q35)
QCPU     := $(if $(filter arm64,$(ARCH)),cortex-a72,qemu64)

KERNEL_SRC := /Volumes/KernelBuild/linux-6.1.112
BUILD    := build/$(ARCH)
KERNEL   := $(BUILD)/$(KIMAGE)
INITRAMFS := $(BUILD)/initramfs.cpio.gz
DAEMON   := target/$(TARGET)/release/aginx-daemon
ROOTFS   := $(BUILD)/rootfs
MODDIR   := $(ROOTFS)/lib/modules

MAKE     := gmake
NPROC    := $(shell sysctl -n hw.ncpu)
PORT     := 9091

.PHONY: all daemon kernel rootfs image run run-arm64 run-x86 clean
.PHONY: kernel-pixel5 boot-redfin flash-redfin vbmeta-disable

all: image

# ── Rust daemon (zigbuild for musl static linking) ──

daemon: $(DAEMON)

$(DAEMON): aginx-daemon/src/*.rs aginx-daemon/Cargo.toml
	cargo zigbuild --package aginx-daemon --target $(TARGET) --release

# ── Kernel (from custom source tree) ──

kernel: $(KERNEL)

$(KERNEL):
	@echo "=== Getting $(ARCH) kernel ==="
	@mkdir -p $(BUILD)
ifeq ($(ARCH),arm64)
	cd $(KERNEL_SRC) && \
	PATH="/opt/homebrew/bin:$$PATH" \
	HOSTCC="cc" \
	HOSTCFLAGS="-I$(KERNEL_SRC)/include-mac-compat" \
	$(MAKE) ARCH=$(KARCH) CROSS_COMPILE=$(CROSS) -j$(NPROC) $(KIMAGE)
	cp $(KERNEL_SRC)/arch/$(KARCH)/boot/$(KIMAGE) $(KERNEL)
else
	@# x86_64: use pre-built Debian kernel (objtool incompatible with macOS)
	curl -sL -o /tmp/linux-image-amd64.deb \
		'http://ftp.debian.org/debian/pool/main/l/linux-signed-amd64/linux-image-6.1.0-44-amd64_6.1.164-1_amd64.deb'
	@rm -rf /tmp/kernel-extract && mkdir -p /tmp/kernel-extract
	cd /tmp/kernel-extract && ar x /tmp/linux-image-amd64.deb && tar xf data.tar.xz
	cp /tmp/kernel-extract/boot/vmlinuz-6.1.0-44-amd64 $(KERNEL)
	@# Cache e1000 module for initramfs
	@mkdir -p $(BUILD)/modules
	cp /tmp/kernel-extract/lib/modules/6.1.0-44-amd64/kernel/drivers/net/ethernet/intel/e1000/e1000.ko $(BUILD)/modules/
endif
	@echo "Kernel: $$(du -h $(KERNEL) | cut -f1)"

kernel-clean:
	cd $(KERNEL_SRC) && PATH="/opt/homebrew/bin:$$PATH" $(MAKE) ARCH=$(KARCH) CROSS_COMPILE=$(CROSS) clean

kernel-defconfig:
	cd $(KERNEL_SRC) && PATH="/opt/homebrew/bin:$$PATH" $(MAKE) ARCH=$(KARCH) CROSS_COMPILE=$(CROSS) olddefconfig

# ── Initramfs ──

rootfs: $(INITRAMFS)

$(INITRAMFS): $(DAEMON)
	@echo "=== Building $(ARCH) rootfs ==="
	@rm -rf $(ROOTFS)
	@mkdir -p $(ROOTFS)/{dev,proc,sys,tmp,etc,bin,lib/modules}
	cp $(DAEMON) $(ROOTFS)/init
	chmod +x $(ROOTFS)/init
	echo "aginx" > $(ROOTFS)/etc/hostname
ifneq ($(wildcard $(BUILD)/modules/*.ko),)
	cp $(BUILD)/modules/*.ko $(MODDIR)/
endif
	cd $(ROOTFS) && find . | cpio -o -H newc 2>/dev/null | gzip > $(CURDIR)/$(INITRAMFS)
	@echo "Rootfs: $$(du -h $(INITRAMFS) | cut -f1)"

# ── Build everything ──

image: daemon rootfs $(KERNEL)
	@echo ""
	@echo "=== AginxOS $(ARCH) ==="
	@echo "  Kernel:    $(KERNEL) ($$(du -h $(KERNEL) | cut -f1))"
	@echo "  Initramfs: $(INITRAMFS) ($$(du -h $(INITRAMFS) | cut -f1))"
	@echo "  Daemon:    $(DAEMON) ($$(du -h $(DAEMON) | cut -f1))"
	@echo "  Total:     $$(du -ch $(KERNEL) $(INITRAMFS) | tail -1 | cut -f1)"
	@echo ""

# ── Run in QEMU (auto-detect arch) ──

run: image
	$(QEMU_BIN) \
		-machine $(QMACHINE) \
		-cpu $(QCPU) \
		-m 256 \
		-nographic \
		-kernel $(KERNEL) \
		-initrd $(INITRAMFS) \
		-append "console=$(KCONSOLE) rdinit=/init" \
		-netdev user,id=net0,hostfwd=tcp::$(PORT)-:$(PORT) \
		-device e1000,netdev=net0

# Convenience targets
run-arm64:
	$(MAKE) run ARCH=arm64

run-x86:
	$(MAKE) run ARCH=x86_64

# macOS Hypervisor.framework acceleration (arm64 only)
run-fast: image
	$(QEMU_BIN) \
		-machine $(QMACHINE) \
		-accel hvf \
		-cpu max \
		-m 256 \
		-nographic \
		-kernel $(KERNEL) \
		-initrd $(INITRAMFS) \
		-append "console=$(KCONSOLE) rdinit=/init" \
		-netdev user,id=net0,hostfwd=tcp::$(PORT)-:$(PORT) \
		-device e1000,netdev=net0

# With network packet capture
run-pcap: image
	$(QEMU_BIN) \
		-machine $(QMACHINE) \
		-cpu $(QCPU) \
		-m 256 \
		-nographic \
		-kernel $(KERNEL) \
		-initrd $(INITRAMFS) \
		-append "console=$(KCONSOLE) rdinit=/init" \
		-netdev user,id=net0,hostfwd=tcp::$(PORT)-:$(PORT) \
		-device e1000,netdev=net0 \
		-object filter-dump,id=f1,netdev=net0,file=$(BUILD)/network.pcap

# ── Clean ──

clean:
	rm -rf build/
	cargo clean --package aginx-daemon --release

clean-all:
	rm -rf build/
	cargo clean

# ── Native (macOS) build for testing ──

daemon-native:
	cargo build --package aginx-daemon --release

# ── Pixel 5 (Redfin) flash targets ──

REDFIN_BUILD := build/arm64
REDFIN_KERNEL := $(REDFIN_BUILD)/Image-pixel5
REDFIN_BOOT   := $(REDFIN_BUILD)/boot-redfin.img
REDFIN_VBMETA := $(REDFIN_BUILD)/vbmeta-disable.img
REDFIN_ROOTFS_CPIO := $(REDFIN_BUILD)/rootfs.cpio
REDFIN_CMDLINE := console=ttyMSM0,115200n8 earlycon=msm_geni_serial,0x00888000 rdinit=/init

# Build rootfs as uncompressed cpio (for built-in initramfs)
$(REDFIN_ROOTFS_CPIO): $(DAEMON)
	@echo "=== Building Pixel 5 rootfs (cpio) ==="
	@rm -rf $(REDFIN_BUILD)/rootfs-pixel5
	@mkdir -p $(REDFIN_BUILD)/rootfs-pixel5/{dev,proc,sys,tmp,etc,bin,lib/modules,var/run/wpa_supplicant}
	cp $(DAEMON) $(REDFIN_BUILD)/rootfs-pixel5/init
	chmod +x $(REDFIN_BUILD)/rootfs-pixel5/init
	echo "aginx" > $(REDFIN_BUILD)/rootfs-pixel5/etc/hostname
	cd $(REDFIN_BUILD)/rootfs-pixel5 && find . | cpio -o -H newc > $(CURDIR)/$(REDFIN_ROOTFS_CPIO)
	@echo "Rootfs cpio: $$(du -h $(REDFIN_ROOTFS_CPIO) | cut -f1)"

# Build kernel with built-in initramfs for Pixel 5
kernel-pixel5: $(REDFIN_KERNEL)

$(REDFIN_KERNEL): $(REDFIN_ROOTFS_CPIO)
	@echo "=== Building Pixel 5 kernel (initramfs built-in) ==="
	@# Create gmake wrapper (kernel 6.1 needs GNU Make >= 4.4)
	@mkdir -p /tmp/gmake-wrap && echo '#!/bin/sh\nexec gmake "$$@"' > /tmp/gmake-wrap/make && chmod +x /tmp/gmake-wrap/make
	@# Save current config
	cp $(KERNEL_SRC)/.config $(KERNEL_SRC)/.config.saved
	@# Modify config to add initramfs source
	sed -i '' 's|CONFIG_INITRAMFS_SOURCE=.*|CONFIG_INITRAMFS_SOURCE="$(CURDIR)/$(REDFIN_ROOTFS_CPIO)"|' $(KERNEL_SRC)/.config
	@# Build kernel
	cd $(KERNEL_SRC) && \
	PATH="/tmp/gmake-wrap:/opt/homebrew/bin:$$PATH" \
	HOSTCC="cc" \
	HOSTCFLAGS="-I$(KERNEL_SRC)/include-mac-compat" \
	gmake ARCH=arm64 CROSS_COMPILE=aarch64-elf- -j$(NPROC) Image
	cp $(KERNEL_SRC)/arch/arm64/boot/Image $(REDFIN_KERNEL)
	@# Restore original config
	mv $(KERNEL_SRC)/.config.saved $(KERNEL_SRC)/.config
	@echo "Kernel (with initramfs): $$(du -h $(REDFIN_KERNEL) | cut -f1)"

# Create V3 boot.img with ANDROID! magic
boot-redfin: $(REDFIN_BOOT)

$(REDFIN_BOOT): $(REDFIN_KERNEL)
	@echo "=== Creating Pixel 5 boot image ==="
	python3 tools/mkboot_redfin.py boot $(REDFIN_KERNEL) $(REDFIN_BOOT) \
		--cmdline '$(REDFIN_CMDLINE)'
	@echo "Boot image: $$(du -h $(REDFIN_BOOT) | cut -f1)"
	@xxd -l 48 $(REDFIN_BOOT)

# Create disabled vbmeta image
vbmeta-disable: $(REDFIN_VBMETA)

$(REDFIN_VBMETA):
	python3 tools/mkboot_redfin.py vbmeta $(REDFIN_VBMETA)

# Flash to Pixel 5 via fastboot
flash-redfin: boot-redfin vbmeta-disable
	@echo "=== Flashing Pixel 5 ==="
	@echo "Checking fastboot device..."
	fastboot devices
	@echo "Flashing boot_a..."
	fastboot flash boot_a $(REDFIN_BOOT)
	@echo "Disabling AVB verification..."
	fastboot flash vbmeta --disable-verification $(REDFIN_VBMETA)
	@echo "Rebooting..."
	fastboot reboot
	@echo "Done! Check device screen or connect via WiFi/serial."
