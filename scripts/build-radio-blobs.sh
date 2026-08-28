#!/usr/bin/env bash
# Regenerate the radio bring-up payload in .local/radio/ (gitignored).
#
#   rmt_storage  stock /vendor/bin/rmt_storage pulled from the experiment
#                unit, then binary-patched: stock resolves
#                persist.vendor.modem.efs.clean at startup and, when the
#                property is unfindable — always true for us, we have no
#                property service — logs "Detected userdata wiped case"
#                and ZEROES modemst1+modemst2 (observed 2026-08-27). A
#                second call site erases on a multisim-config mismatch.
#                Both are `bl clear_modem_efs_partitions` (fn at 0x5328);
#                the patch NOPs them. Byte offsets verified against this
#                unit's binary only — the script refuses to patch anything
#                that does not still read as those two bl instructions.
#   libnl.so     bionic shared lib cnss-daemon needs (not on the device;
#                built once from AOSP external/libnl — see .local/radio/
#                README.md for the exact build; re-copied if present in
#                /tmp).
#   cdsp-cdsp-loader.ko  stock vendor cdsp-loader.ko, renames ONLY: module
#                name cdsp_loader→cdsp4loader, driver cdsp-loader→aginx-cdsp4,
#                sysfs dir boot_cdsp→cdsp4boot (all same-length, .rodata
#                + symbol strings). compatible and ALL code bytes untouched.
#                Binds soc:qcom,msm-cdsp-loader (property
#                qcom,proc-img-to-load = "cdsp") → subsystem_get("cdsp") —
#                the pure stock code path. Booted the CDSP on the first
#                try (2026-08-28) after three nop-patch attempts crashed:
#                NEVER nop a call site that carries a relocation — the
#                loader re-applies R_AARCH64_CALL26 at 0x310 over the nop
#                and the result executes as brk #0x100 → do_undefinstr
#                panic (captured in /var/crash-cdsp.log 2026-08-27).
#   modem-npucc-loader.ko  the modem trigger, re-anchored so it can
#                coexist with the CDSP loader (each needs a bound DT node
#                to create its sysfs attr; both matched qcom,cdsp-loader).
#                Same proven two edits as the old cdsp-modem-loader, PLUS
#                two same-or-shorter .rodata string edits:
#                  0x25b  "cdsp\0"→"modem\0"   subsystem_get() target
#                  0x1314 cbnz w0 → nop        strcmp-result check
#                  0x350  compatible (32-byte field) → "qcom,lito-npucc"
#                  0x1d6  property name "qcom,proc-img-to-load"(22B
#                         slot) → "compatible" — the npucc node HAS one,
#                         so of_property_read_string succeeds and no
#                         rc-check/strcmp bypass is needed.
#                Loads AFTER the CDSP loader (it no longer competes for
#                the cdsp node; npucc is otherwise unbound).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${ROOT}/.local/radio"
DEV="${DEV:-aginxosredfin}"
NDK="${NDK:-$HOME/Library/Android/sdk/ndk/27.0.12077973}"
OBJDUMP="${NDK}/toolchains/llvm/prebuilt/darwin-x86_64/bin/llvm-objdump"

mkdir -p "${OUT}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

adb -s "${DEV}" pull /vendor/bin/rmt_storage "${work}/rmt_storage" >/dev/null
orig_size=$(stat -f %z "${work}/rmt_storage")
echo "pulled rmt_storage (${orig_size} bytes)"

# Patch sites: (offset, expected bl target 0x5328), little-endian storage
# order as xxd -p emits it (objdump displays these as 9400024a / 940001a2).
declare -a SITES=(0x4a00 0x4ca0)
declare -a EXPECT=(4a020094 a2010094)
cp "${work}/rmt_storage" "${work}/patched"
for i in 0 1; do
  off=$((SITES[$i]))
  word=$(dd if="${work}/patched" bs=1 skip="${off}" count=4 status=none | xxd -p)
  if [ "${word}" != "${EXPECT[$i]}" ]; then
    echo "offset ${SITES[$i]}: expected ${EXPECT[$i]}, found ${word}" >&2
    echo "rmt_storage differs from the analyzed build — re-derive offsets" \
         "before patching (see scripts/ and HARDWARE.md M3d)" >&2
    exit 1
  fi
  printf '\x1f\x20\x03\xd5' | dd of="${work}/patched" bs=1 seek="${off}" \
    conv=notrunc status=none
  echo "patched ${SITES[$i]}: bl 0x5328 -> nop"
done
chmod 755 "${work}/patched"
"${OBJDUMP}" -d --start-address=0x4a00 --stop-address=0x4a04 "${work}/patched" | grep -q "nop" \
  || { echo "post-patch disasm check failed" >&2; exit 1; }
cp "${work}/patched" "${OUT}/rmt_storage"
echo "-> ${OUT}/rmt_storage"

# --- cdsp-modem-loader.ko: retarget stock CDSP boot module to the modem ---
# Expected bytes at each site (little-endian); refuse on any drift.
check_word() {  # file offset expected_hex what
  got=$(dd if="$1" bs=1 skip="$2" count=$(( ${#3} / 2 )) status=none | xxd -p)
  [ "${got}" = "$3" ] || {
    echo "$4 at 0x$(printf %x "$2"): expected $3, found ${got}" >&2
    echo "cdsp-loader.ko differs from the analyzed build — re-derive offsets first" >&2
    exit 1
  }
}
patch_bytes() {  # file offset hexstr
  printf "%s" "$3" | xxd -r -p | dd of="$1" bs=1 seek="$2" conv=notrunc status=none
}
adb -s "${DEV}" pull /vendor_a/lib/modules/cdsp-loader.ko "${work}/cdsp-loader.ko" >/dev/null
check_word "${work}/cdsp-loader.ko" 0x25b  636473700063 "subsystem name"  # "cdsp\0c"
check_word "${work}/cdsp-loader.ko" 0x1314 60040035     "cbnz w0"         # 35000460 LE
check_word "${work}/cdsp-loader.ko" 0x1d6  71636f6d2c70726f632d696d672d746f2d6c6f616400 "prop name"

# ---- modem-npucc-loader.ko: modem trigger anchored on the npucc node ----
cp "${work}/cdsp-loader.ko" "${work}/modem-npucc.ko"
patch_bytes "${work}/modem-npucc.ko" 0x25b  6d6f64656d00                  # "modem\0"
patch_bytes "${work}/modem-npucc.ko" 0x1314 1f2003d5                      # nop
patch_bytes "${work}/modem-npucc.ko" 0x350  71636f6d2c6c69746f2d6e7075636300 # qcom,lito-npucc\0
patch_bytes "${work}/modem-npucc.ko" 0x1d6  636f6d70617469626c6500          # compatible\0
cp "${work}/modem-npucc.ko" "${OUT}/modem-npucc-loader.ko"
echo "-> ${OUT}/modem-npucc-loader.ko"

# ---- cdsp-cdsp-loader.ko: pure CDSP loader, renames only ----------------
python3 - "${work}/cdsp-loader.ko" "${OUT}/cdsp-cdsp-loader.ko" <<'PYEOF'
import sys
d = bytearray(open(sys.argv[1], 'rb').read())
def patch_str(off, old, new):
    assert len(new) <= len(old), (old, new)
    assert bytes(d[off:off+len(old)]).startswith(old), (hex(off), bytes(d[off:off+len(old)]))
    d[off:off+len(old)] = new + bytes(len(old) - len(new))
patch_str(0x251, b'boot_cdsp\0', b'cdsp4boot\0')     # sysfs dir
patch_str(0x226, b'cdsp-loader\0', b'aginx-cdsp4\0') # driver name
assert d.count(b'cdsp_loader') == 12
d = bytearray(bytes(d).replace(b'cdsp_loader', b'cdsp4loader'))
open(sys.argv[2], 'wb').write(d)
PYEOF
echo "-> ${OUT}/cdsp-cdsp-loader.ko"

if [ -f /tmp/libnl.so ]; then
  cp /tmp/libnl.so "${OUT}/libnl.so"
  echo "-> ${OUT}/libnl.so (from /tmp/libnl.so)"
else
  echo "NOTE: /tmp/libnl.so absent — copy an existing build or rebuild" \
       "from AOSP external/libnl (instructions in ${OUT}/README.md)" >&2
  [ -f "${OUT}/libnl.so" ] || exit 1
fi
echo "done: $(ls -l "${OUT}" | tail -2 | awk '{print $NF, $5}')"
