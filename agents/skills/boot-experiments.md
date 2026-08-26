# Boot Experiments

Read this before flashing a test image, verifying boot behavior, or recovering
from a bootloop. This is the "did the splash actually paint / did Android come
back" procedure — compile success is not verification.

## The verification loop

A bring-up change goes through this loop, never a shortcut:

1. **Pack** — build the artifact (`./scripts/pack-vendor-boot.sh` or the boot.img pipeline).
2. **Flash** — get it on the experiment unit via fastboot.
3. **Observe** — watch the screen and collect logs.
4. **Record** — append the *observed* result to `docs/HARDWARE.md`.
5. **Restore** — end in a known-good state.

"Expect green → red → blue → yellow" is a prediction. It becomes a result only
after step 4 records what was actually seen.

## Flashing a vendor_boot test

The full pack-flash-watch-recover cycle is scripted:

```bash
# Try splash with modules, hand off to Android after:
HOLD=0 MODULES=drm ./scripts/flash-early-splash.sh

# Hold in our pid1 (no Android handoff) and paint colors:
HOLD=1 MODULES=drm ./scripts/flash-early-splash.sh
```

`flash-early-splash.sh` packs, waits for fastboot, flashes
`boot/out/vendor_boot-test.img`, reboots, then polls `adb`/`fastboot`:

- prints `ANDROID_OK` and pulls `dmesg | grep -i aginxos` plus ramoops if
  Android returns
- auto-restores stock `vendor_boot` if it sees fastboot come back twice (bootloop)

The `HOLD` flag controls whether we hand off to Android:

- `HOLD=1` — our pid1 keeps running; Android should **not** appear. A stable
  stuck Google logo with no USB *is* the success signal that our pid1 is alive.
- `HOLD=0` — the trampoline hands off to `first_stage_init`; Android should
  eventually boot.

## What counts as success on screen

- **Color frames** (green → red → blue → yellow, white border + corner block)
  only appear if DRM got a usable CRTC. On the stock kernel this usually does
  *not* happen — early `msm_drm` load from ramdisk doesn't produce a modeset.
- **Google logo stuck** (no modeset) + stable = our pid1 is holding, but the
  display path isn't painting. Log it as such; do not report "splash works".
- **Android boots** = handoff succeeded. Check `adb shell dmesg | grep -i aginxos`
  for the trampoline's kmsg lines to confirm our code actually ran.

Do not infer paint from kmsg alone. `DRM splash OK` in kmsg means the ioctls
returned success, not that pixels were visibly correct — the screen is the source
of truth.

## Collecting logs

Trampoline kmsg is the primary bring-up signal (see
[`agents/skills/vendor-boot.md`](vendor-boot.md) for the message vocabulary).

```bash
# From a booted Android, after a handoff attempt:
adb shell dmesg | grep -i aginxos

# If the device bootlooped, pstore may have kept the last messages:
adb shell cat /sys/fs/pstore/console-ramoops-0 2>/dev/null | grep -i aginxos | tail -50
```

## Recovering from a bootloop

The known-good restore is always stock `vendor_boot`:

```bash
./scripts/restore-vendor-boot.sh
# manual equivalent:
#   fastboot flash vendor_boot boot/stock-vendor_boot.img && fastboot reboot
```

If the device is stuck and won't respond to adb: force reboot into fastboot
(Volume Down + Power), then flash stock. The factory image in `.factory/` is the
last resort for a full `flash-all` wipe.

## Recording the result

Append one row to the relevant table in `docs/HARDWARE.md` with date, artifact,
and observed outcome. Keep the "expected" vs "observed" distinction explicit —
a row that says "confirm on device" is an unfinished experiment, not a result.
