#!/usr/bin/env bash
# Boot health smoke (M29): the boot.state spine, provision's layers, the
# supervised units, and the /var/bin provisioned faces. Read-only — run
# this first when picking up the device; it answers "is the phone
# healthy right now" without caring which milestone is under test.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

# ---- boot.state spine ------------------------------------------------------
drv 'test -f /run/boot.state'
expect_rc 0 'boot.state exists'

drv 'grep -q "^internet ok" /run/boot.state'
expect_rc 0 'net-bringup reached internet ok'

drv 'grep -q "^pkg ok" /run/boot.state'
expect_rc 0 'provision resync ok'

drv 'grep -q "^done ok" /run/boot.state'
expect_rc 0 'net-bringup closed with done ok'

# ---- provision layers ------------------------------------------------------
drv 'test -f /var/tmp/agpkg-sync.log'
expect_rc 0 'agpkg sync log exists'

drv 'test -d /var/lib/ag/done && test -d /var/lib/agpkg/skills && test -d /var/lib/agpkg/units'
expect_rc 0 'seed dirs present'

# ---- units -----------------------------------------------------------------
drv 'agctl list'
expect_rc 0 'agctl list runs'
expect_out 'ready' 'at least one unit ready'

# ---- provisioned faces -----------------------------------------------------
drv 'ls /var/bin | wc -l'
expect_rc 0 '/var/bin enumerable'
if [ "${DRV_OUT}" -ge 5 ] 2>/dev/null; then
  t_ok '/var/bin has faces (>=5)'
else
  t_fail '/var/bin has faces (>=5)'
fi

drv 'uptime >/dev/null'
expect_rc 0 'device responsive'

suite_done
