# boot/rootfs — rootfs recipe

Templates assembled into the ext4 rootfs image by `scripts/build-rootfs.sh`
(flashed to `userdata`, entered via `switch_root` — see `docs/HARDWARE.md`
v0.7.0 for the proven boot sequence).

- `busybox` — busybox 1.36.1 (git 1a64f6a), `make defconfig` +
  `CONFIG_STATIC=y`, cross-built `aarch64-linux-musl` via `zig cc`.
  Serves as `/bin/busybox`, `/sbin/init`, and the applet symlinks; rcS runs
  `busybox --install -s /bin` on first boot to fill in the full applet set.
- `etc/` — inittab, rcS, and the adbd respawn wrapper. These are the exact
  files proven on device; the comment in `inittab` was updated after the
  respawn was enabled. `etc/init.d/adbd` re-binds the USB gadget UDC after
  each adbd death — that teardown is why a plain respawn isn't enough.

The Android half (`/system`, `default.prop`, `*_contexts`) is copied from
`boot/out/vendor-ramdisk-root/` at build time, not stored here.

mke2fs records the building user's uid as owner; rcS normalizes to 0:0 on
first boot.
