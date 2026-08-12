# initramfs sources

| File | Purpose |
|------|---------|
| `init` | First userspace process in the test boot.img |
| `busybox` | Optional aarch64 static busybox (gitignored if you add it) |
| `aginxos-probe` | Optional: copy musl probe here before `pack-boot.sh` |

```bash
# Optional: embed the musl probe in the ramdisk
./scripts/build-phone.sh musl probe
cp target/aarch64-unknown-linux-musl/release/aginxos-probe boot/initramfs/
./boot/pack-boot.sh
```

Obtain static busybox (example):

```text
https://busybox.net/downloads/binaries/
# or build: make ARCH=arm64 CROSS_COMPILE=...
```

Place the binary at `boot/initramfs/busybox` (aarch64, preferably static).
