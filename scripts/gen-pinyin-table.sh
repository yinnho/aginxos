#!/bin/bash
# Generate crates/aterm/data/pinyin.tsv — the M40 pinyin IME table.
#
# Two upstream sources, both fetched to /tmp (never committed — only the
# derived compact table ships):
#   - mozillazg/pinyin-data pinyin.txt — hanzi -> pinyin readings (MIT)
#   - Jun Da Modern Chinese Character Frequency List — rank order
#
# Derivation: readings are tone-stripped (NFD, drop combining marks, ü->v),
# restricted to the GB2312 hanzi set (the exact enumeration
# scripts/subset-cjk-font.sh baked into agterm-cjk.otf — the font is the
# coverage constraint), and ranked per syllable by Jun Da frequency. Top 12
# candidates per syllable. One line per syllable, tab-separated:
#   ni	你尼泥妮…
set -euo pipefail

Pinyin_URL="https://raw.githubusercontent.com/mozillazg/pinyin-data/master/pinyin.txt"
FREQ_URL="http://lingua.mtsu.edu/chinese-computing/statistics/char/download.php?Which=MO"
P="/tmp/m40-pinyin.txt"
F="/tmp/m40-junda.csv"
OUT="$(dirname "$0")/../crates/aterm/data/pinyin.tsv"

if [ ! -f "$P" ]; then
  curl -sL --max-time 300 -o "$P" "$Pinyin_URL" \
    || curl -sL --max-time 300 -o "$P" "https://gh-proxy.com/$Pinyin_URL"
fi
if [ ! -f "$F" ]; then
  curl -sL --max-time 300 -o "$F" "$FREQ_URL"
fi

mkdir -p "$(dirname "$OUT")"
python3 - "$P" "$F" "$OUT" <<'EOF'
import re, sys, unicodedata

def toneless(s):
    # NFD + drop tone marks; the diaeresis survives as ü so nü/lü stay
    # distinct from nu/lu, then becomes the typed form v (nv/lv/nve)
    s = unicodedata.normalize('NFD', s)
    s = ''.join(c for c in s
                if not unicodedata.combining(c) or c == '̈')
    return unicodedata.normalize('NFC', s).replace('ü', 'v')

# GB2312 hanzi set — same enumeration as scripts/subset-cjk-font.sh
gb = set()
for b1 in range(0xB0, 0xF8):
    for b2 in range(0xA1, 0xFF):
        try: gb.add(bytes([b1, b2]).decode('gb2312'))
        except UnicodeDecodeError: pass

# readings: char -> set of toneless syllables
syl2chars = {}
for line in open(sys.argv[1], encoding='utf-8'):
    m = re.match(r'U\+([0-9A-F]+): (.+?)  #', line)
    if not m: continue
    ch = chr(int(m.group(1), 16))
    if ch not in gb: continue
    for r in m.group(2).split(','):
        s = toneless(r.strip().lower())
        if re.fullmatch(r'[a-z]+', s):
            syl2chars.setdefault(s, set()).add(ch)

# Jun Da rank (1 = most frequent); chars missing from the list rank last
rank = {}
for line in open(sys.argv[2], encoding='gbk', errors='ignore'):
    if line.startswith('/*'): continue
    f = line.rstrip('\r\n').split('\t')
    if len(f) >= 2 and f[0].isdigit():
        rank[f[1]] = int(f[0])

out = []
for syl in sorted(syl2chars):
    chars = sorted(syl2chars[syl], key=lambda c: rank.get(c, 1 << 30))
    if chars:
        out.append(syl + '\t' + ''.join(chars[:12]))

open(sys.argv[3], 'w', encoding='utf-8').write('\n'.join(out) + '\n')
pairs = sum(len(l.split('\t')[1]) for l in out)
print(f'{len(out)} syllables, {pairs} candidates')
EOF

ls -la "$OUT"
