#!/bin/bash
# Subset Noto Sans Mono CJK SC for aterm (M38a).
#
# Produces boot/rootfs/usr/share/fonts/agterm-cjk.otf — the font aterm's
# cjk.rs rasterizes through ab_glyph. Coverage: full GB2312 (6763 hanzi,
# level 1+2 — agent chat is arbitrary text, level 1's 3755 leaves common
# tech chars like 渲 missing) + ASCII + GB2312 A1/A3 rows (CJK punctuation,
# fullwidth forms). Mono face on purpose: half-width Latin + 2x full-width
# hanzi advance are exactly terminal cell semantics (docs/DEVICE.md
# 软件侧 #3 — ab_glyph + Noto subset, no hinting, no fontdue).
#
# Input font is fetched to /tmp (never committed — 16MB upstream; only the
# ~1.5MB OFL-licensed subset ships in the repo).
set -euo pipefail

SRC_URL="https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/Mono/NotoSansMonoCJKsc-Regular.otf"
SRC="/tmp/NotoSansMonoCJKsc-Regular.otf"
OUT="$(dirname "$0")/../boot/rootfs/usr/share/fonts/agterm-cjk.otf"
CS="/tmp/cjk-charset.txt"

if [ ! -f "$SRC" ]; then
  echo "fetching $SRC_URL"
  curl -sL --max-time 600 -o "$SRC" "$SRC_URL" \
    || curl -sL --max-time 600 -o "$SRC" "https://gh-proxy.com/$SRC_URL"
fi

python3 - "$CS" <<'EOF'
import sys
chars = set(chr(c) for c in range(0x20, 0x7F))
for row in (0xA1, 0xA3):
    for b2 in range(0xA1, 0xFF):
        try: chars.add(bytes([row, b2]).decode('gb2312'))
        except UnicodeDecodeError: pass
for b1 in range(0xB0, 0xF8):  # GB2312 level 1+2 hanzi
    for b2 in range(0xA1, 0xFF):
        try: chars.add(bytes([b1, b2]).decode('gb2312'))
        except UnicodeDecodeError: pass
chars |= set('，。、；：？！“”‘’（）《》【】…—·～￥％°℃〇')
open(sys.argv[1], 'w').write(''.join(sorted(chars)))
print(len(chars), 'chars')
EOF

mkdir -p "$(dirname "$OUT")"
python3 -m fontTools.subset "$SRC" \
  --text-file="$CS" \
  --output-file="$OUT" \
  --no-hinting --desubroutinize \
  --name-IDs=1,2,3,4,6 --glyph-names \
  --notdef-outline --recommended-glyphs \
  --layout-features=''

ls -la "$OUT"
