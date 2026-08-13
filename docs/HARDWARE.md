# AginxOS — Pixel 5 (redfin) hardware log

Fill this in during bring-up. Do not invent nodes; only record what the device shows.

## Device

| Field | Value |
|-------|--------|
| Codename | redfin |
| SoC | SM7250 (Snapdragon 765G) |
| Bootloader | unlocked |
| Role | experimental |
| Serial | 13201FDD4001N8 |
| Current slot | a |
| Stock / last known good | factory `UP1A.231105.001.B2` (Android 14) |

## Probe output

```
AginxOS probe 0.1.0
uid=2000
kernel: Linux version 4.19.278-g7b0944645172-ab10812814 ... #1 SMP PREEMPT Thu Sep 7 05:43:03 UTC 2023
hostname: <unavailable: Permission denied (os error 13)>
input: /dev/input → event0, event1, event2, event3
dri: /dev/dri → card0, renderD128
```

Captured: 2026-08-13 via `./scripts/push-probe-android.sh` on stock Android userspace.

## Nodes

| Subsystem | Path / notes | Status |
|-----------|----------------|--------|
| Input | `/dev/input/event0`–`event3` | present (shell uid 2000) |
| DRM | `/dev/dri/card0`, `renderD128` | present |
| WLAN | | not probed yet |
| Modem / QRTR | | not probed yet |
| Firmware dir | extract from vendor locally | not yet |

## Boot experiments

| Date | boot.img | Result |
|------|----------|--------|
| 2026-08-13 | stock factory flash complete | Android boots; adb OK |
| 2026-08-13 | minimal ramdisk (`aginxos-init` only) | bootloader OK; black Google logo hang (no modules/display path). Recover via force reboot + stock boot flash. |
| 2026-08-13 | hybrid ramdisk (stock + wrap `/init` → DRM splash → `/init.android`) | `fastboot boot` sent OKAY; expect green splash ~4s then Android. Confirm on device. |
