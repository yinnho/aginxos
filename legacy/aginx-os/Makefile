# Aginx OS Makefile
#
# Build system for Aginx OS (Agent OS for Pixel 5)

# Toolchain
CROSS ?= aarch64-unknown-aginx
RUSTC ?= rustc
CARGO ?= cargo

# Directories
KERNEL_DIR = kernel
SCHEMES_DIR = schemes
DRIVERS_DIR = drivers
LIBS_DIR = libs
BUILD_DIR = build
TARGET_DIR = target

# Build target
TARGET = aarch64-aginx.json

# QEMU settings
QEMU = qemu-system-aarch64
QEMU_MACHINE = virt
QEMU_CPU = cortex-a76
QEMU_MEM = 512M

# Default target
.PHONY: all
all: kernel schemes drivers

# Build kernel
.PHONY: kernel
kernel:
	@echo "Building kernel..."
	$(CARGO) build --package aginx-kernel --release --target $(TARGET)

# Build userspace programs
.PHONY: userspace
userspace:
	@echo "Building userspace programs..."
	@mkdir -p out
	rustc --edition 2021 --target aarch64-unknown-none --crate-type bin \
		-C link-arg=-Tuserspace/linker.ld -C link-arg=-nostdlib \
		-C panic=abort -C opt-level=s -o out/hello.elf userspace/hello.rs
	@# Convert ELF to flat binary using a simple approach
	@# (Requires llvm-objcopy or manual extraction)
	@echo "Built out/hello.elf — use 'exec hello.elf' or convert to .bin"

# Build all schemes
.PHONY: schemes
schemes: init ion smolnetd sshd

.PHONY: init ion smolnetd sshd
init ion smolnetd sshd:
	@echo "Building $@..."
	$(CARGO) build --package $@ --release --target $(TARGET)

# Build all drivers
.PHONY: drivers
drivers: driver-network pcid ath11kd

.PHONY: driver-network pcid ath11kd
driver-network pcid ath11kd:
	@echo "Building $@..."
	$(CARGO) build --package $@ --release --target $(TARGET)

# Build everything
.PHONY: build
build:
	$(CARGO) build --release --target $(TARGET)

# Create bootable image
.PHONY: image
image: build
	@echo "Creating bootable image..."
	mkdir -p $(BUILD_DIR)
	# TODO: Create initramfs
	# TODO: Create disk image

# Run in QEMU
.PHONY: qemu
qemu: kernel
	$(QEMU) -machine $(QEMU_MACHINE),highmem=off \
		-cpu $(QEMU_CPU) \
		-m $(QEMU_MEM) \
		-kernel $(TARGET_DIR)/aarch64-aginx/release/aginx-kernel \
		-nographic \
		-serial mon:stdio \
		-netdev user,id=net0,hostfwd=tcp::9091-:9091,hostfwd=tcp::9092-:9092 \
		-device virtio-net-pci-non-transitional,netdev=net0 \
		-drive file=disk.img,format=raw,if=none,id=blk0 \
		-device virtio-blk-pci,drive=blk0

# Clean build artifacts
.PHONY: clean
clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)

# Check code
.PHONY: check
check:
	$(CARGO) check --target $(TARGET)

# Format code
.PHONY: fmt
fmt:
	$(CARGO) fmt

# Run clippy
.PHONY: clippy
clippy:
	$(CARGO) clippy --target $(TARGET)

# Help
.PHONY: help
help:
	@echo "Aginx OS Build System"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Build kernel, schemes, and drivers"
	@echo "  kernel    - Build kernel only"
	@echo "  schemes   - Build all schemes"
	@echo "  drivers   - Build all drivers"
	@echo "  build     - Build everything"
	@echo "  image     - Create bootable disk image"
	@echo "  qemu      - Run in QEMU emulator"
	@echo "  clean     - Remove build artifacts"
	@echo "  check     - Check code without building"
	@echo "  fmt       - Format source code"
	@echo "  clippy    - Run clippy linter"
