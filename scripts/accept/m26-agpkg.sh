#!/usr/bin/env bash
# M26 acceptance — agpkg 四件套 + signed manifest, offline halves only:
# list/available read local state; sync is deliberately NOT run here
# (network + downloads have their own one-off experiments; the tamper
# rejection was device-proven in M26 and re-running it would mean
# editing /etc mid-suite).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

# ---- manifest + signature --------------------------------------------------
drv 'test -f /etc/agpkg.manifest -a -f /etc/agpkg.manifest.sig'
expect_rc 0 'manifest and detached sig present'

drv 'grep -qE "^aginx " /etc/agpkg.manifest'
expect_rc 0 'aginx pinned in manifest'

# ---- installed state through the router ------------------------------------
drv 'ag pkg list --json'
expect_rc 0 'ag pkg list --json runs'
expect_py 'pkg list is a D1 envelope' 'j.get("ok") is True and isinstance(j.get("data"), list)'
expect_py 'aginx installed' 'any(x.get("name") == "aginx" for x in j["data"])'

# ---- available reads the signed manifest -----------------------------------
drv 'ag pkg available --json'
expect_rc 0 'ag pkg available --json runs'
expect_py 'available is a D1 envelope' 'j.get("ok") is True'

# ---- skills landed (四件套 tars keep their doc next to the install;
# bare v0 binaries have none — python3 is the tree-bundle proof) ------
drv 'test -f /var/lib/agpkg/skills/python3/SKILL.md'
expect_rc 0 'python3 SKILL.md present'

suite_done
