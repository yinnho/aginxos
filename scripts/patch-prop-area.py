#!/usr/bin/env python3
"""Build a minimal patched bionic property area for the rdinit env.

Layout (reverse-engineered from this device's own /dev/__properties__/ areas,
2026-08-27; matches bionic libc/private/system_properties.h):
  prop_info = { char value[92]; char name[] }  with a 4-byte serial at
  name-96, encoded (little-endian read) as (value_len << 24) | count << 16.

Both patches are same-length single-byte value swaps, so the serial field is
left untouched. Output dir holds only what bionic needs to resolve the
patched context: properties_serial, property_info (the serialized contexts
trie), and the one context area we patch. Missing context files simply
resolve to "property not found" (ContextNode::Open fails lazily, per-node).
"""
import pathlib
import shutil
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "boot/out/props"
DST = ROOT / "boot/out/props-min"

# (context file, property name, expected current value, new value)
PATCHES = [
    ("u:object_r:userdebug_or_eng_prop:s0", "ro.debuggable", b"0", b"1"),
    ("u:object_r:userdebug_or_eng_prop:s0", "ro.secure", b"1", b"0"),
]
# ro.debuggable=1 -> adbd's __android_log_is_debuggable() is false -> no RSA
# auth handshake. ro.secure=0 -> should_drop_privileges() keeps adbd root.


def patch_area(blob: bytes, name: str, old: bytes, new: bytes) -> bytes:
    if len(old) != len(new):
        sys.exit(f"{name}: only same-length patches are safe (serial len byte)")
    n = blob.find(name.encode() + b"\0")
    if n < 0:
        sys.exit(f"{name}: not found in area")
    voff = n - 92
    soff = n - 96
    serial = struct.unpack_from("<I", blob, soff)[0]
    if (serial >> 24) != len(old):
        sys.exit(f"{name}: serial len byte {serial >> 24} != {len(old)} — layout drift")
    if blob[voff : voff + len(old)] != old:
        sys.exit(f"{name}: current value {blob[voff:voff+len(old)]!r} != {old!r}")
    blob = bytearray(blob)
    blob[voff : voff + len(new)] = new
    return bytes(blob)


def main() -> None:
    for meta in ("properties_serial", "property_info"):
        if not (SRC / meta).is_file():
            sys.exit(f"missing {SRC / meta} — run scripts/pull-prop-area.sh first")
    shutil.rmtree(DST, ignore_errors=True)
    DST.mkdir(parents=True)
    for meta in ("properties_serial", "property_info"):
        shutil.copyfile(SRC / meta, DST / meta)
    by_ctx: dict[str, list[tuple[str, bytes, bytes]]] = {}
    for ctx, name, old, new in PATCHES:
        by_ctx.setdefault(ctx, []).append((name, old, new))
    for ctx, plist in by_ctx.items():
        blob = (SRC / ctx).read_bytes()
        for name, old, new in plist:
            blob = patch_area(blob, name, old, new)
            print(f"patched {name}: {old!r} -> {new!r} in {ctx}")
        (DST / ctx).write_bytes(blob)
    print(f"minimal area in {DST}:")
    for p in sorted(DST.iterdir()):
        print(f"  {p.name}  {p.stat().st_size} bytes")


if __name__ == "__main__":
    main()
