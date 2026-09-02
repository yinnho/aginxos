#!/usr/bin/env bash
# M24 acceptance — the ag router on device (the plan's 验收单, made
# re-runnable). The headline is the Omarchy-accident class: --help on a
# destructive command must intercept BEFORE the target runs, proven by
# an unchanged uptime, not just by an exit code.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

# ---- interception: destructive command, target never runs -------------------
drv 'cut -d. -f1 /proc/uptime'
U1="${DRV_OUT}"

drv 'ag sys reboot --help'
expect_rc 0 'ag sys reboot --help intercepts (rc 0)'
expect_out 'usage' 'help text shows usage'

drv 'cut -d. -f1 /proc/uptime'
if [ "${DRV_OUT}" -ge "${U1}" ] 2>/dev/null; then
  t_ok 'uptime monotonic — reboot2 never ran'
else
  t_fail 'uptime monotonic — reboot2 never ran'
fi

# ---- bare call with required args ------------------------------------------
drv 'ag snd-cap'
expect_rc 2 'bare ag snd-cap refused with usage (rc 2)'
expect_out 'usage' 'usage line present'

# ---- unknown command -------------------------------------------------------
drv 'ag nope'
expect_rc 127 'unknown command exits 127'
expect_out "unknown command 'nope'" 'unknown-command message'
expect_out 'did you mean|ag ' 'suggestion offered'

# ---- registry --------------------------------------------------------------
drv 'ag commands --check'
expect_rc 0 'ag commands --check green'
expect_out 'commands OK' 'lint line'

drv 'ag commands --json'
expect_rc 0 'ag commands --json runs'
expect_py 'commands --json is a D1 envelope' 'j.get("ok") is True and isinstance(j["data"], list) and j["meta"]["count"] > 0'

drv 'ag commands'
expect_rc 0 'menu renders'

# ---- dispatch through a shim to a real binary ------------------------------
drv 'ag done list'
expect_rc 0 'ag done list dispatches to agdone'

suite_done
