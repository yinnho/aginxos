#!/usr/bin/env bash
# Build the AginxOS ext4 rootfs image for the userdata partition (M2).
#
# Assembles a tree from: the unpacked vendor ramdisk (/system + Android prop
# and SELinux context files adbd needs), boot/rootfs/ (busybox + /etc), and
# the musl release binaries. Then packs it with mke2fs -d.
#
# Flash with:  fastboot flash userdata out/rootfs.img
# Boot needs a vendor_boot packed with ROOTFS=1 (scripts/pack-vendor-boot.sh).
#
# Note: mke2fs -d records the building user's uid (501 on macOS) as owner.
# rcS chowns everything back to 0:0 on first boot — do not "fix" that here.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RAMDISK="${ROOT}/boot/out/vendor-ramdisk-root"
RECIPE="${ROOT}/boot/rootfs"
TARGET="${ROOT}/target/aarch64-unknown-linux-musl/release"
TREE="${TREE:-/tmp/aginxos-rootfs}"
IMG="${IMG:-${ROOT}/out/rootfs.img}"
# 2 GB sparse-ish image: codex alone is 223 MB, and the fs is flashed onto
# the 114 GB userdata partition (grow with resize2fs later if ever needed).
SIZE="${SIZE:-2g}"

test -x "${RAMDISK}/system/bin/adbd" || { echo "missing ${RAMDISK} — run boot/unpack-boot.sh first" >&2; exit 1; }
test -x "${RECIPE}/busybox" || { echo "missing ${RECIPE}/busybox" >&2; exit 1; }
test -x "${TARGET}/aginxos-init" || { echo "missing musl binaries — run scripts/build-phone.sh musl first" >&2; exit 1; }
test -x "${TARGET}/aterm" || { echo "missing aterm — run scripts/build-phone.sh musl first" >&2; exit 1; }
test -x "${TARGET}/ag" || { echo "missing ag router — run scripts/build-phone.sh musl first" >&2; exit 1; }
MKE2FS="$(command -v mke2fs || true)"
test -z "${MKE2FS}" && MKE2FS=/opt/homebrew/bin/mke2fs
test -x "${MKE2FS}" || { echo "mke2fs not found (android-platform-tools provides it)" >&2; exit 1; }

rm -rf "${TREE}"
mkdir -p "${TREE}"

# Mountpoints (and /var/log — the only place boot evidence survives; the
# kernel has no pstore, so /var/adbd.log is our cross-boot record).
# /var/power + /var/lib/agpkg/{skills,units,stamps} + /var/lib/ag/done:
# state-tar members that only existed on the running fs before bake #9 —
# busybox tar exits 1 on a missing member, which the hardened agupd
# rightly treats as fatal (observed 2026-09-03). /var/lib/ag/done is the
# agdone marker home (M27); provision seeds it at runtime too.
# /var/tmp — NOT tmpfs (only /tmp is), yet nothing created it: bake #10's
# fresh image shipped without it, provision's `>$LOG` redirect failed and
# resync reported pkg-fail-with-no-log (observed 2026-09-03). agupd also
# stage-builds its state tar there (M22 note).
mkdir -p "${TREE}"/{dev,proc,sys,etc,home,media,mnt,opt,root,run,srv,tmp,var/log,var/power,var/tmp}
mkdir -p "${TREE}"/var/lib/agpkg/{skills,units,stamps}
mkdir -p "${TREE}"/var/lib/ag/done

# Android pieces: /system (adbd + linker config + lib64) and the root-level
# property/SELinux files adbd reads at startup.
cp -R "${RAMDISK}/system" "${TREE}/system"
for f in default.prop prop.default *_contexts; do
  cp "${RAMDISK}"/${f} "${TREE}/" 2>/dev/null || true
done

# Kernel modules for the touch/display chain (M3) — the ramdisk half. The
# vendor_boot base loads only the 64-module USB/storage set (modules.usb);
# the full modules.load load panics this kernel (observed 2026-08-27, retry
# counter burned), so the touch chain is loaded from the rootfs world by
# /etc/init.d/touch-bringup, in the order proven live. Same .ko files as
# the ramdisk holds — copied from the local unpack (never committed, §7).
MODULES="spi-geni-qcom rpmsg_core qrtr qrtr-smd ion-alloc qseecom \
hdcp_qseecom msm_hdcp msm_ext_display llcc-slice dispcc-lito \
qpnp-amoled-regulator msm_drm"
# Battery chain (M3c) — loaded by /etc/init.d/battery-bringup. Order
# matters: google-bms provides gbms_storage, at24 registers the
# batt_eeprom entry qpnp-qgauge's probe reads, qpnp-qgauge registers the
# "bms" psy google-battery waits on. qti_qmi_sensor rides along last
# (charge mitigation; needs qmi_helpers from the vendor half).
MODULES="${MODULES} google-bms at24 qpnp-qgauge sm7250_bms google-battery \
google_charger qti_qmi_sensor"
mkdir -p "${TREE}/lib/modules"
for m in ${MODULES}; do
  cp "${RAMDISK}/lib/modules/${m}.ko" "${TREE}/lib/modules/"
done

# DRM splash painter — the panel stays black without an explicit mode set
# (the bootloader logo is cont-splash scanout, not KMS; connector sits at
# enabled=disabled). touch-bringup paints green when touch is up. Built with
# the same zig toolchain as the trampoline. binder-init mounts binderfs and
# allocates the binder/hwbinder/vndsbinder devices cnss-daemon needs (this
# kernel's backport ioctl struct — see the source header).
ZIG="$(command -v zig || true)"
test -z "${ZIG}" && ZIG=/opt/homebrew/bin/zig
test -x "${ZIG}" || { echo "zig not found (needed for splash2/binder-init)" >&2; exit 1; }
mkdir -p "${TREE}/bin"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/splash" "${RECIPE}/src/splash2.c"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/binder-init" "${RECIPE}/src/binder-init.c"
# QRTR observability (M3d): qrtr-lookup snapshots/watches the name service,
# qmi-req sends one raw QMI request. radio-bringup starts a qrtr-lookup
# watcher before the modem boot trigger to record the fresh-boot service
# registration order (WLFW 0x45 transient vs never-present).
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/qrtr-lookup" "${RECIPE}/src/qrtr-lookup.c"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/qmi-req" "${RECIPE}/src/qmi-req.c"
# cam-shot (M19) — the IFE/RDI stills capture tool. Vendor sensor register
# tables are decoded into the source; vendor module bins stay local and
# gitignored. Default flags for a rear shot: --stream --rear --slowrear
# --rawvendor [--gain N] [--png].
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/cam-shot" "${RECIPE}/src/cam-shot.c"
# raw2jpg (M19c) — RAW10 dump -> JPEG converter, companion to cam-shot's
# native --jpeg (for converting already-captured dumps).
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/raw2jpg" "${RECIPE}/src/raw2jpg.c"

# Bionic LD_PRELOAD helpers (M3d). These load into vendor binaries, so they
# must be NDK/bionic shared objects, not musl. trace_open.so mirrors file
# access AND logcat output (__android_log_print & co) onto stderr — it is
# our only window into cnss-daemon/pd-mapper, which log exclusively through
# liblog and we run no logd. fake-props.so fakes the servicemanager
# properties pm-service blocks on and logs every other property read.
NDK_CC="${HOME}/Library/Android/sdk/ndk/27.0.12077973/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android24-clang"
test -x "${NDK_CC}" || { echo "NDK clang not found (needed for preload .so)" >&2; exit 1; }
mkdir -p "${TREE}/lib"
"${NDK_CC}" -shared -fPIC -O2 -o "${TREE}/lib/trace_open.so" "${RECIPE}/src/trace_open.c"
"${NDK_CC}" -shared -fPIC -O2 -o "${TREE}/lib/fake-props.so" "${RECIPE}/src/fake-props.c"
echo "built preload helpers (trace_open.so, fake-props.so)"
# fake-sm: minimal binder context manager (musl-static) answering every
# transaction with Status-ok. Without a CM on /dev/binder, vendor libbinder
# clients (cnss-daemon, pm-service) spin forever in "Waiting 1s on context
# object" before ever reaching their QMI work.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/fake-sm" "${RECIPE}/src/fake-sm.c"
# reboot2: raw reboot(LINUX_REBOOT_CMD_RESTART2) — toybox reboot signals init
# (we run none) and adb reboot needs adbd's sys.powerctl handling. With no
# args it plain-reboots; "bootloader" lands in fastboot for re-flashing.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/reboot2" "${RECIPE}/src/reboot2.c"
# wdt (M20c): watchdog probe/arm/starve for /dev/watchdog. The dog itself
# is armed and petted by agsvc (crates/agsvc); this is the diagnostics
# tool that proved the platform story (softdog behind msm_watchdog,
# hardware bark resources absent) and the live-fire starve reset.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/wdt" "${RECIPE}/src/wdt.c"
# rtcal (M23b): pm8xxx RTC alarm arm/read + `sync` — the suspend probes' wake
# path ("set <epoch>" semantics kept from the /tmp zig one-off). net-bringup
# runs `rtcal sync` after ntpd to fix the RTC's -53y offset, which also makes
# early-boot wall time true on the next HCTOSYS pass.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/rtcal" "${RECIPE}/src/rtcal.c"
# Ops channel sshd (#142, 2026-09-04) — the maintenance face: ssh in over
# Wi-Fi or `adb forward tcp:2222 tcp:22` when the subnets differ. Static
# musl dropbear triplet from scripts/build-dropbear.sh (zig cc; the AR must
# be LLVM's — macOS BSD ar archives break lld member resolution). rcS starts
# the daemon key-only with the host key under /root/.ssh.
DROPBEAR="${ROOT}/.local/dropbear/bin"
for b in dropbear dbclient dropbearkey; do
  test -x "${DROPBEAR}/${b}" || { echo "missing ${b} — run scripts/build-dropbear.sh" >&2; exit 1; }
  cp "${DROPBEAR}/${b}" "${TREE}/bin/${b}"
  chmod 755 "${TREE}/bin/${b}"
done
# nlscan: nl80211 trigger-scan + dump client — busybox has no wireless tools
# and we ship no libnl. Our WLAN operability check (M3f).
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/nlscan" "${RECIPE}/src/nlscan.c"
# Wi-Fi join (M4): self-contained WPA2-PSK supplicant — CONNECT, EAPOL 4-way
# handshake over an AF_PACKET socket, NEW_KEY installs; then udhcpc owns IP
# provisioning. wifi-trace flips QCA vendor dp-trace levels for TX/RX logs.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/wifi-join" "${RECIPE}/src/wifi-join.c"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/wifi-trace" "${RECIPE}/src/wifi-trace.c"
# M18 audio I/O: bare-ioctl PCM pair (no alsa-lib) — capture is the
# agent's "listen" path, playback its "speak" path. Shared uapi header.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/snd-cap" "${RECIPE}/src/snd-cap.c"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/snd-play" "${RECIPE}/src/snd-play.c"
# snd-mixer: ctl get/set (no alsa-lib) — audio-bringup's whole routing
# recipe runs through it. i2c-reg: rt5514 register peek/poke over
# /dev/i2c-N (kernel has no debugfs here — see audio-bringup notes).
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/snd-mixer" "${RECIPE}/src/snd-mixer.c"
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/i2c-reg" "${RECIPE}/src/i2c-reg.c"
# Boot card (M5): DRM boot-status renderer — polls /run/boot.state and
# paints the AginxOS bring-up checklist on the panel. Holds DRM master
# for its whole life (it replaces the M3 green splash). Same zig static
# build; host-side layout check via `bootcard --ppm out.ppm [state]`.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/bootcard" "${RECIPE}/src/bootcard.c"
# httpget: minimal HTTP fetch for the boot internet check — busybox's wget
# applet segfaults in this build (2026-08-28), so net-bringup uses ours.
"${ZIG}" cc -target aarch64-linux-musl -static -O2 \
  -o "${TREE}/bin/httpget" "${RECIPE}/src/httpget.c"
# udhcpc event hook (compiled-in default path) — without it udhcpc wins a
# lease but nothing applies it to the interface.
mkdir -p "${TREE}/usr/share/udhcpc"
cp "${RECIPE}/usr/share/udhcpc/default.script" "${TREE}/usr/share/udhcpc/"
chmod 755 "${TREE}/usr/share/udhcpc/default.script"

# Radio bring-up payload (M3d). libnl.so is the bionic build cnss-daemon
# dlopens (LD_LIBRARY_PATH=/lib/...); rmt_storage is the PATCHED stock
# binary (erase call sites NOPed); cdsp-cdsp-loader.ko is stock
# cdsp-loader.ko with module/driver/sysfs names renamed (compat + code
# untouched — it binds soc:qcom,msm-cdsp-loader and boots the CDSP) and
# modem-npucc-loader.ko is the modem variant re-anchored to the npucc
# node. See .local/radio/README.md. All are vendor-derived blobs: they
# live only in gitignored .local/radio/ and are copied in when present.
# scripts/build-radio-blobs.sh regenerates them.
RADIO="${ROOT}/.local/radio"
if [ -f "${RADIO}/libnl.so" ] && [ -x "${RADIO}/rmt_storage" ] \
   && [ -f "${RADIO}/cdsp-cdsp-loader.ko" ] \
   && [ -f "${RADIO}/modem-npucc-loader.ko" ]; then
  mkdir -p "${TREE}/lib" "${TREE}/lib/modules"
  cp "${RADIO}/libnl.so" "${TREE}/lib/libnl.so"
  cp "${RADIO}/rmt_storage" "${TREE}/bin/rmt_storage"
  chmod 755 "${TREE}/bin/rmt_storage"
  cp "${RADIO}/cdsp-cdsp-loader.ko" "${RADIO}/modem-npucc-loader.ko" \
     "${TREE}/lib/modules/"
  echo "staged radio payload (libnl.so + patched rmt_storage + cdsp/modem loaders)"
else
  echo "NOTE: .local/radio incomplete — radio-bringup will fail; run scripts/build-radio-blobs.sh" >&2
fi

# Our pieces.
mkdir -p "${TREE}/bin" "${TREE}/sbin" "${TREE}/aginxos"
cp "${RECIPE}/busybox" "${TREE}/bin/busybox"
cp -R "${RECIPE}/etc/." "${TREE}/etc/"
# version stamp (M14): what the running image is, for agupd status/compare.
{ git -C "${ROOT}" log -1 --format="aginxos %h %cd" --date=short 2>/dev/null || echo "aginxos unknown"; } > "${TREE}/etc/aginx-version"
mkdir -p "${TREE}/usr/bin"
cp -R "${RECIPE}/usr/bin/." "${TREE}/usr/bin/"
# agdl (M10) — the only working HTTPS fetcher on the phone; agpkg sync and
# the first-boot provisioner download required-tier software with it.
cp "${TARGET}/agdl" "${TREE}/usr/bin/agdl"
# aterm (M11) — the on-device terminal UI: launcher + pty shell on the panel.
cp "${TARGET}/aterm" "${TREE}/usr/bin/aterm"
# voiced (M42a) — the voice dialog daemon: closed-vocabulary deterministic
# protocol (no LLM needed to get on Wi-Fi), PTT on volume-down, brain
# ASR/TTS, and the sole writer of /run/voice/face which aterm renders.
cp "${TARGET}/voiced" "${TREE}/usr/bin/voiced"
# CJK font subset (M38a) — aterm cjk.rs rasterizes through ab_glyph;
# GB2312 full + ASCII + punct rows, ~1.5MB (scripts/subset-cjk-font.sh).
cp -R "${RECIPE}/usr/share/fonts" "${TREE}/usr/share/fonts"
# wifi-wizard (M10) — first-boot Wi-Fi setup TUI (scan/pick/password),
# auto-started by aterm when /etc/wifi.conf is missing.
cp "${TARGET}/wifi-wizard" "${TREE}/usr/bin/wifi-wizard"
# service layer (M16) — agsvc supervisor (inittab respawns it; units in
# /etc/agsvc.d + /var/lib/agpkg/units), agctl control client over
# /run/svc/ctl.sock, and agboot-ok which marks the slot boot-successful
# so ABL's retry counter stops draining us into fastboot.
cp "${TARGET}/agsvc" "${TARGET}/agctl" "${TARGET}/agboot-ok" "${TREE}/usr/bin/"
# updater (M14) — A/B self-update: pulls a manifest, writes the inactive
# slot, flips it active (agboot-ok set-active), reboots; rollback rides on
# ABL's tries counter as above.
cp "${TARGET}/agupd" "${TREE}/usr/bin/agupd"
# agpkg (M26) — package installer, Rust rewrite of the sh v0: signed
# manifest chain (same ed25519 key as agupd) + 四件套 tars (bin/<name> +
# pkg.toml + SKILL.md). The recipe's sh agpkg is gone; this binary is
# the only face.
cp "${TARGET}/agpkg" "${TREE}/usr/bin/agpkg"
# agdone (M27) — provision done-markers for one-shot boot steps
# (/var/lib/ag/done). The ag-done shim rides along via the cp -R above.
cp "${TARGET}/agdone" "${TREE}/usr/bin/agdone"
# agsecret (M36) — the secret sidecar: agsecretd daemon (unit in
# /etc/agsvc.d) + agsecret admin client (router face via the ag-secret
# shim in the recipe). Store lands at /var/lib/ag/secret/store at first
# put; policy is the tracked /etc/aginx/secret.policy.
cp "${TARGET}/agsecretd" "${TARGET}/agsecret" "${TREE}/usr/bin/"
# package manifest rides SIGNED: the on-device default path requires a
# detached sig or every `agpkg sync` refuses (fail-closed). Resign when
# the manifest is newer than its sig; no key = hard error, not a silent
# unsigned image.
# Content-based check (git does not carry mtimes): resign when the sig
# is missing, or when it no longer verifies against the manifest (a
# machine with the pubkey can prove staleness; without it we ship the
# committed sig as-is — it is a public artifact, safe to commit).
AGPKG_KEY=".local/keys/agupd.key"
AGPKG_PUB=".local/keys/agupd.pub"
AGPKG_MF="${RECIPE}/etc/agpkg.manifest"
AGPKG_SIG="${RECIPE}/etc/agpkg.manifest.sig"
AGPKG_NEED_SIGN=0
[[ -f "${AGPKG_SIG}" ]] || AGPKG_NEED_SIGN=1
if [[ "${AGPKG_NEED_SIGN}" -eq 0 && -f "${AGPKG_PUB}" ]] \
   && ! cargo run -q -p agsign -- verify "${AGPKG_PUB}" "${AGPKG_MF}" >/dev/null 2>&1; then
  AGPKG_NEED_SIGN=1
fi
if [[ "${AGPKG_NEED_SIGN}" -eq 1 ]]; then
  [[ -f "${AGPKG_KEY}" ]] || { echo "FATAL: ${AGPKG_MF} needs signing but ${AGPKG_KEY} is missing" >&2; exit 1; }
  cargo run -q -p agsign -- sign "${AGPKG_KEY}" "${AGPKG_MF}"
  echo "==> signed ${AGPKG_MF}"
fi
# ag (M24) — single-entry command router: flat ag-* universe, filename is
# the route, filesystem is the registry. The ag-* sh shims in the recipe
# ride along via the cp -R above; groups.desc in /etc/ag.
cp "${TARGET}/ag" "${TREE}/usr/bin/ag"
chmod 755 "${TREE}/etc/init.d/rcS" "${TREE}/etc/init.d/adbd" \
  "${TREE}/etc/init.d/touch-bringup" "${TREE}/etc/init.d/battery-bringup" \
  "${TREE}/etc/init.d/radio-bringup" "${TREE}/etc/init.d/audio-bringup" \
  "${TREE}/etc/init.d/net-bringup" \
  "${TREE}/etc/init.d/app-registry" "${TREE}/etc/init.d/provision" \
  "${TREE}/etc/init.d/aterm-handoff" \
  "${TREE}/etc/init.d/camera-bringup" "${TREE}/etc/init.d/cell-bringup" \
  "${TREE}/etc/init.d/state-restore" \
  "${TREE}/usr/bin/agpkg" "${TREE}/usr/bin/agdone" "${TREE}/usr/bin/agdl" "${TREE}/usr/bin/aterm" \
  "${TREE}/usr/bin/voiced" \
  "${TREE}/usr/bin/wifi-wizard" "${TREE}/usr/bin/agsvc" \
  "${TREE}/usr/bin/agctl" "${TREE}/usr/bin/agboot-ok" \
  "${TREE}/usr/bin/agupd" \
  "${TREE}/usr/bin/agsecretd" "${TREE}/usr/bin/agsecret" \
  "${TREE}/usr/bin/ag" \
  "${TREE}/usr/bin/net-rejoin" "${TREE}/usr/bin/net-watch" \
  "${TREE}/bin/wdt"
# ag-* sh shims (M24): recipe files may not carry the exec bit through
# git/cp, and a non-executable shim is invisible to the router.
for s in "${TREE}/usr/bin/"ag-*; do chmod 755 "${s}"; done
# NB: wifi.conf.example rides along in ${RECIPE}/etc — the real
# /etc/wifi.conf (with the passphrase) is pushed by hand, never committed.
cp "${TARGET}/aginxos-init" "${TARGET}/aginxos-agent" "${TREE}/aginxos/"

# Curated applet symlinks — enough for init and debugging; rcS runs
# `busybox --install -s /bin` to fill in the full set on first boot.
APPLETS="[ awk blkid cat chmod chown clear cp cut date dd df dmesg echo env \
expr false fdisk find free getty grep gunzip gzip head hostname id insmod ip \
kill less ln ls lsmod mkdir mknod more mount mv netcat netstat nice passwd \
pidof ping printf ps renice rm rmdir route sed setsid sh sleep sort \
start-stop-daemon stat su switch_root sync tail tar telnet test top touch tr \
true umount uname uniq uptime vi wc wget which whoami xargs zcat"
for a in ${APPLETS}; do ln -sf busybox "${TREE}/bin/${a}"; done
ln -sf ../bin/busybox "${TREE}/sbin/init"
ln -sf ../bin/busybox "${TREE}/sbin/reboot"
ln -sf ../bin/busybox "${TREE}/sbin/poweroff"
ln -sf ../bin/busybox "${TREE}/sbin/ifconfig"

# TLS trust store: codex (and anything using system-native cert roots)
# fails with "waiting for network" without it. Cached under out/ so the
# download happens once per host, not once per build.
CACERT="${ROOT}/out/cacert.pem"
if [ ! -s "${CACERT}" ]; then
  curl -sL --max-time 120 -o "${CACERT}" https://curl.se/ca/cacert.pem
fi
mkdir -p "${TREE}/etc/ssl/certs"
cp "${CACERT}" "${TREE}/etc/ssl/certs/ca-certificates.crt"
ln -sf certs/ca-certificates.crt "${TREE}/etc/ssl/cert.pem"

# Registry gate (M24): lint the assembled command set with a host-built
# ag before the image is packed. AG_CMD_PATH points at the tree's bin dirs
# so ag:exec targets baked into /bin resolve; /var/bin is device-provisioned
# (absent here), which is why the carrier pass-through shims declare no
# ag:exec. Fails the build on collisions, missing summaries, bad metadata,
# or missing exec targets.
cargo build -p ag --release >/dev/null
AG_CMD_PATH="${TREE}/usr/bin:${TREE}/bin:${TREE}/sbin" \
AG_GROUPS_DESC="${TREE}/etc/ag/groups.desc" \
  "${ROOT}/target/release/ag" commands --check \
  || { echo "ag commands --check failed — fix the shims" >&2; exit 1; }

mkdir -p "${ROOT}/out"
# rm first: mke2fs never truncates an existing output file, so a SIZE
# change leaves stale bytes past the new fs end (a 2g image stayed 2 GiB
# after re-baking at 1g — the tail was the old image, 2026-09-02).
rm -f "${IMG}"
"${MKE2FS}" -t ext4 -b 4096 -F -d "${TREE}" "${IMG}" "${SIZE}"
echo "built ${IMG} ($(du -h "${IMG}" | cut -f1)) from ${TREE}"
