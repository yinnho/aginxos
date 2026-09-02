#!/usr/bin/env bash
# M25 acceptance — HOME unified on /home and D1 envelopes on the carrier
# faces. The aterm-side $HOME assertion stays manual (on-screen); the
# adb-checkable halves are here.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

# ---- HOME unification ------------------------------------------------------
drv 'grep -q "^HOME=/home" /etc/aginx/env'
expect_rc 0 'unit env pins HOME=/home'

drv 'test -d /home/.aginx'
expect_rc 0 '/home/.aginx state present'

# ---- carrier faces speak the envelope --------------------------------------
# Faces may print a human hint line first; the agio envelope is the
# compact last line (expect_py scans for it). cron list has no --json
# flag — the envelope is its default output shape.
drv 'ag agent list --json'
expect_rc 0 'ag agent list --json runs'
expect_py 'agent list is a D1 envelope' 'j.get("ok") is True and isinstance(j.get("data"), list)'

drv 'ag cron list'
expect_rc 0 'ag cron list runs'
expect_py 'cron list is a D1 envelope' 'j.get("ok") is True'

suite_done
