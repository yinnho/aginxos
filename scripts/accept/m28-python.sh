#!/usr/bin/env bash
# M28 acceptance — python3 core tier: the tree bundle installed by sync,
# the loader finalize that made it exec'able, and pip on musllinux
# wheels. Read-only (pip installs nothing here — that was the one-off
# `pip install six` experiment recorded in HARDWARE.md).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

# ---- the face is a symlink into the package tree ---------------------------
drv 'test -x /var/bin/python3 -a -L /var/bin/python3'
expect_rc 0 '/var/bin/python3 is an executable symlink face'

drv 'test -d /var/lib/agpkg/pkgfiles/python3'
expect_rc 0 'tree bundle landed in pkgfiles'

# ---- the finalize tenant ---------------------------------------------------
drv 'test -e /lib/ld-musl-aarch64.so.1'
expect_rc 0 'musl loader linked into /lib'

drv 'test -f /var/lib/ag/done/python-finalize'
expect_rc 0 'python-finalize marker present'

# ---- the interpreter works -------------------------------------------------
drv 'python3 -V'
expect_rc 0 'python3 runs'
expect_out 'Python 3\.12' '3.12 line'

drv 'python3 -c "import ssl, sqlite3, socket, json; print(1)"'
expect_rc 0 'core modules import'

drv 'python3 -m pip --version'
expect_rc 0 'pip present'

# ---- visible through the package manager -----------------------------------
drv 'ag pkg list --json'
expect_py 'pkg list shows python3' 'any(x.get("name") == "python3" for x in j["data"])'

# ---- the system guarantee line ---------------------------------------------
drv 'test -f /var/lib/agpkg/skills/_system/SKILL.md'
expect_rc 0 '_system skill seeded'

drv 'grep -q "python3" /var/lib/agpkg/skills/_system/SKILL.md'
expect_rc 0 'guarantee line mentions python3'

suite_done
