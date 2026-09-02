#!/usr/bin/env bash
# Shared helpers for the acceptance suites (M29). Source, never run:
#   . "$(dirname "$0")/lib.sh"
#
# Safety contract: every adb call anywhere in a suite goes through
# adbx(), pinned to the experiment unit's serial — the daily phone must
# never be touched by an acceptance run. Suites are read-only plus the
# explicitly named scratch markers they clean up after themselves;
# nothing reboots, flashes, or writes outside /var/lib/ag/done/accept-*
# and /var/tmp.

# The experiment unit (docs/HARDWARE.md). NOT the daily phone.
ADB_SERIAL="aginxosredfin"

PASS=0
FAIL=0

adbx() { command adb -s "${ADB_SERIAL}" "$@"; }

suite_require_device() {
  local state
  state="$(adbx get-state 2>/dev/null || true)"
  if [ "${state}" != "device" ]; then
    echo "FAIL: experiment unit ${ADB_SERIAL} not attached (state: ${state:-none})" >&2
    exit 2
  fi
}

# drv '<shell-cmd>' — run on device, capture exit code and stdout.
# Sets DRV_RC / DRV_OUT. PATH mirrors provision's so /var/bin faces and
# /usr/bin tools resolve. HOME is pinned to /home — adbd spawns shells
# with HOME=/ (M30 bring-up finding), which made the carrier CLI carve
# a stray registry out of /.aginx instead of the daemon's /home/.aginx;
# /etc/aginx/env only reaches units, not adb. The trailing __RC= echo
# survives adb's historic non-propagation of device exit codes. The
# bionic linker noise lines ("libc: Access denied finding property …")
# that this adb injects into every shell stream are stripped — same
# filter the bring-up sessions have always used by hand.
drv() {
  local raw
  raw="$(adbx shell "export HOME=/home PATH=/usr/bin:/bin:/sbin:/var/bin; $1; echo __RC=\$?" 2>&1 || true)"
  raw="${raw//$'\r'/}"
  DRV_RC="$(printf '%s\n' "${raw}" | sed -n 's/^__RC=//p' | tail -1)"
  # `|| true`: a command with no stdout empties the last grep -v, which
  # exits 1 — under set -e/pipefail that must not kill the suite.
  DRV_OUT="$(printf '%s\n' "${raw}" | grep -vE '^(libc:|linker|WARNING)' | grep -v '^__RC=' || true)"
}

# assertions -----------------------------------------------------------------

t_ok()   { echo "ok   - $1"; PASS=$((PASS + 1)); }
t_fail() {
  echo "FAIL - $1"
  echo "       rc=${DRV_RC:-?} out=$(printf '%s' "${DRV_OUT:-}" | head -1)"
  FAIL=$((FAIL + 1))
}

expect_rc() {
  local want="$1" name="$2"
  if [ "${DRV_RC}" = "${want}" ]; then t_ok "${name}"; else t_fail "${name} (want rc ${want})"; fi
}

expect_out() {
  local pat="$1" name="$2"
  if printf '%s\n' "${DRV_OUT}" | grep -qE "${pat}"; then t_ok "${name}"; else t_fail "${name} (want /${pat}/)"; fi
}

# The last JSON-parseable line of DRV_OUT satisfies <expr> (bound as j).
# Carrier faces legitimately print a human hint line before the envelope
# (agio emits the machine face compact on one line, last). Uses the
# HOST python3 — the device one is itself under test in m28.
expect_py() {
  local name="$1" expr="$2"
  if printf '%s\n' "${DRV_OUT}" | python3 -c \
      "import json,sys
j = None
for ln in sys.stdin:
    try:
        j = json.loads(ln)
    except ValueError:
        pass
sys.exit(0 if j is not None and (${expr}) else 1)" 2>/dev/null; then
    t_ok "${name}"
  else
    t_fail "${name} (${expr})"
  fi
}

suite_done() {
  echo "---"
  echo "${PASS} passed, ${FAIL} failed"
  [ "${FAIL}" -eq 0 ]
}
