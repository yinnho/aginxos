#!/usr/bin/env bash
# M27 acceptance — agdone marker discipline + provision v2 layers, made
# re-runnable. Uses only the scratch marker accept-m27 (and a wedge dir
# it removes again); real markers like python-finalize are only ever
# read here.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=accept-m27

# ---- marker lifecycle on the real CLI --------------------------------------
drv "agdone check ${M}"
expect_rc 3 'unmarked checks rc 3 (a query answer, not a failure)'

drv "agdone mark ${M}"
expect_rc 0 'mark succeeds'

drv "agdone check ${M}"
expect_rc 0 'marked checks rc 0'

# rc discipline under --json: an UNMARKED name still exits 3 (scripts
# branch on rc) while JSON consumers get their ok:true envelope with
# marked:false — check that on a name that was never marked.
drv "agdone check ${M}-json --json"
expect_rc 3 'json check keeps rc 3 when unmarked'
expect_py 'json envelope answers marked:false' 'j.get("ok") is True and j["data"]["marked"] is False'

drv 'agdone list'
expect_out "${M}" 'list shows the marker'

# ---- bad marker reads as unmarked ------------------------------------------
drv "mkdir -p /var/lib/ag/done/${M}-wedge"
drv "agdone check ${M}-wedge"
expect_rc 3 'directory squatting the path reads unmarked'
drv "rmdir /var/lib/ag/done/${M}-wedge"

# ---- names are file names --------------------------------------------------
drv 'agdone mark ../escape'
expect_rc 2 'traversal name refused with usage'

# ---- cleanup ----------------------------------------------------------------
drv "agdone reset ${M}"
expect_rc 0 'reset removes the scratch marker'
drv "agdone check ${M}"
expect_rc 3 'marker gone'

# ---- provision v2 ----------------------------------------------------------
drv 'grep -q "^pkg run" /run/boot.state && grep -q "^pkg ok" /run/boot.state'
expect_rc 0 'boot.state records the resync layer'

drv 'test -f /var/tmp/agpkg-sync.log'
expect_rc 0 'sync output landed in its log'

drv 'test -d /var/lib/ag/done'
expect_rc 0 'seed layer dir present'

suite_done
