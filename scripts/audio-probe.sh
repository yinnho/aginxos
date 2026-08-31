#!/bin/sh
# audio-probe — one-shot M18 audio-chain snapshot, run on the phone:
#   adb -s aginxosredfin shell sh -s < scripts/audio-probe.sh
# Prints sound cards + PCM nodes, ADSP/APR state (qrtr name service),
# the kernel audio log, and vendor audio modules present vs loaded.
# Output feeds a docs/HARDWARE.md entry — observed results only.
set -u

say() { echo; echo "=== $* ==="; }

say "sound cards"
if [ -d /proc/asound ]; then
	cat /proc/asound/cards 2>/dev/null
	echo "--- pcms"
	cat /proc/asound/pcm 2>/dev/null
	echo "--- devices"
	ls -l /dev/snd/ 2>/dev/null
else
	echo "no /proc/asound — no sound card registered"
fi

say "adsp state"
for f in /sys/kernel/boot_adsp /sys/kernel/boot_modem; do
	echo -n "$f: "; cat "$f" 2>/dev/null || echo "(absent)"
done

say "qrtr name service (4s sweep — APR/audio services show here)"
/bin/qrtr-lookup 0 0 4 2>&1 | sed 's/^/  /'

say "kernel audio log"
dmesg | grep -iE 'adsp|q6|apr|bolero|wcd|swr|asoc|snd|audio|pcm|soundwire' \
	| tail -60

say "audio modules loaded"
lsmod | grep -iE 'q6|apr|snd|audio|bolero|wcd|swr|usf|pinctrl.*lpi' || \
	echo "(none)"

say "vendor audio modules available (not necessarily loaded)"
V=/vendor/lib/modules
[ -d "$V" ] || V=/vendor_a/lib/modules
ls "$V" | grep -iE 'q6|apr|snd|audio|bolero|wcd|swr|usf' || echo "(none in $V)"

say "done"
