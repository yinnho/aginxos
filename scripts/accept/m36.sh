#!/usr/bin/env bash
# m36 acceptance — secret sidecar (M36, DECISIONS §9 / SYSTEM.md §7.1).
# Verifies on the real wire: daemon up + socket perms, admin face round
# trip (stdin values), per-scope authz via the real /proc peer path,
# policy live reload, the carrier's sidecar env leg (api hmac resolution
# boundary), log hygiene, and the ag-backup store exclusion.
set -euo pipefail
. "$(dirname "$0")/lib.sh"
suite_require_device

SCRATCH=/var/tmp/accept-m36
POLICY=/etc/aginx/secret.policy
STAGE="$(mktemp -d)"

cleanup() {
  # device back to pre-suite state: policy bytes, no scratch scopes, no scratch dir
  drv "[ -f $SCRATCH/policy.bak ] && cp $SCRATCH/policy.bak $POLICY"
  drv "ag secret rm m36.ak" || true
  drv "ag secret rm m36.sk" || true
  drv "ag secret rm m36.probe" || true
  drv "rm -rf $SCRATCH"
  rm -rf "$STAGE"
}
trap cleanup EXIT

drv "rm -rf $SCRATCH && mkdir -p $SCRATCH"
drv "cp $POLICY $SCRATCH/policy.bak"

# --- 1. unit + socket -------------------------------------------------------

drv 'HOME=/home /usr/bin/agctl list | grep agsecretd'
expect_out 'agsecretd.+ready' 'agsecretd unit ready'

drv 'ls -l /run/aginx/secret.sock'
expect_out '^srw------- .*secret\.sock' 'socket present, 0600'

drv 'ls -ld /run/aginx'
expect_out '^drwx------ ' '/run/aginx 0700'

# --- 2. admin face round trip ----------------------------------------------

drv 'echo -n m36val | ag secret set m36.probe'
expect_py 'put envelope ok' 'j["ok"] is True and j["data"]["scope"] == "m36.probe"'

drv 'ls -l /var/lib/ag/secret/store'
expect_out '^-rw------- ' 'store file 0600'

drv 'ag secret list'
expect_py 'list shows scope' 'j["ok"] is True and "m36.probe" in j["data"]'

drv 'ag secret list'
! printf '%s' "${DRV_OUT}" | grep -q m36val && t_ok "list leaks no value" || t_fail "list leaks no value"

drv 'ag secret get m36.probe'
expect_rc 1 'get from admin exe denied (rc)'
expect_out '"code":"denied"' 'get from admin exe denied (envelope)'

drv 'ag secret env AGINXBRAIN_API_KEY'
expect_rc 1 'env consumer op from admin exe denied (rc)'
expect_out '"code":"denied"' 'env consumer op from admin exe denied (envelope)'

# --- 3. policy live reload --------------------------------------------------
# append-only variant staged on host, pushed, applied WITHOUT a restart.

cat > "$STAGE/policy.probe.json" <<'EOF'
{
  "env": {
    "AGINXBRAIN_API_KEY": "brain.primary",
    "CHARTER_SK": "api.charter",
    "AGINX_M36_AK": "m36.ak",
    "AGINX_M36_SK": "m36.sk"
  },
  "allow": {
    "brain.primary": ["/var/bin/aginx-carrier"],
    "api.charter": ["/var/bin/aginx-carrier"],
    "m36.ak": ["/var/bin/aginx-carrier"],
    "m36.sk": ["/var/bin/aginx-carrier"],
    "m36.probe": ["/usr/bin/agsecret"],
    "admin": ["/usr/bin/agsecret"]
  }
}
EOF
adbx push "$STAGE/policy.probe.json" "$SCRATCH/policy.probe" >/dev/null
drv "cp $SCRATCH/policy.probe $POLICY"

drv 'ag secret get m36.probe'
expect_py 'live reload: same scope now allowed' 'j["ok"] is True and j["data"]["value"] == "m36val"'

drv "cp $SCRATCH/policy.bak $POLICY"
drv 'ag secret get m36.probe'
expect_rc 1 'live reload: restore denies again (rc)'

# --- 4. carrier sidecar leg -------------------------------------------------
# Scratch api tool whose hmac creds resolve from nowhere on call 1
# ("not configured"), then from policy+store on call 2 (gets past
# resolution, dies on the dead port instead). Proves env face for the
# allowed exe /var/bin/aginx-carrier on the real SO_PEERCRED path.

cat > "$STAGE/probe.toml" <<'EOF'
[[tool]]
name = "m36probe"
description = "M36 acceptance probe (scratch)"
method = "GET"
url = "http://127.0.0.1:1/x"

[tool.hmac]
key_id_env = "AGINX_M36_AK"
secret_env = "AGINX_M36_SK"
sign_template = "{method}\n{path}\n{timestamp}\n{body}"
EOF
adbx push "$STAGE/probe.toml" "$SCRATCH/probe.toml" >/dev/null

drv "ag api call m36probe --json --toml $SCRATCH/probe.toml </dev/null"
expect_rc 1 'unmapped creds: not configured (rc)'
expect_out "key_id_env 'AGINX_M36_AK' not configured" 'unmapped creds: not configured (message)'

# allow policy + store values, same dead URL
drv "cp $SCRATCH/policy.probe $POLICY"
drv 'echo -n m36ak | ag secret set m36.ak'
drv 'echo -n m36sk | ag secret set m36.sk'

drv "ag api call m36probe --json --toml $SCRATCH/probe.toml </dev/null"
expect_rc 1 'sidecar-filled creds: past resolution (rc)'
if printf '%s' "${DRV_OUT}" | grep -qiE 'refus|error sending|connect|timed out'; then
  t_ok "sidecar-filled creds: transport error, not 'not configured'"
else
  t_fail "sidecar-filled creds: transport error, not 'not configured'"
fi

drv "cp $SCRATCH/policy.bak $POLICY"

# --- 5. hygiene -------------------------------------------------------------

drv "grep -c m36val /var/log/agsecretd.log"
expect_out '^0$' 'log never contains values'

drv 'grep -c "op=" /var/log/agsecretd.log'
[ "${DRV_OUT}" -ge 1 ] 2>/dev/null && t_ok "log records ops" || t_fail "log records ops"

drv 'ag-backup now'
expect_rc 0 'ag-backup now rc'

drv 'ls -t /var/backups/aginx/backup-*.tar.gz | head -1'
NEWEST="$(printf '%s' "${DRV_OUT}" | head -1)"
drv "tar tzf $NEWEST | grep -c 'var/lib/ag/secret'"
expect_out '^0$' 'backup tar excludes the secret store'

# --- 6. router face ---------------------------------------------------------

drv 'ag secret --help'
expect_rc 0 'ag secret --help intercept'
expect_out 'agsecret' 'intercept shows metadata'

drv 'ag commands --check'
expect_rc 0 'ag commands --check'

suite_done
