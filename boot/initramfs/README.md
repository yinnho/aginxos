# initramfs sources

| File | Purpose |
|------|---------|
| `aginxos-init` | Static aarch64 ELF `/init` (from `cargo zigbuild -p aginxos-init`) |
| `aginxos-probe` | Optional probe binary copied into `/bin` |
| `init` | Legacy shell script fallback (needs busybox) |
| `busybox` | Optional aarch64-only helpers |

```bash
./scripts/build-boot-test.sh
adb reboot bootloader
fastboot boot boot/out/boot-test.img
```

Do **not** use 32-bit ARM busybox; the phone is aarch64.
