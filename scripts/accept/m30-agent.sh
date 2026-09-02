#!/usr/bin/env bash
# M30 acceptance — 化身注册面 + dup 上机（首个四件套包）。
#
# Covers: dup 四件套安装产物（face/skill/无单元）、离线本地环
# （init→commit→log→status）、agent install --file 本地 tar 源
# （--dry-run 预检、真装、list --json 信封、workspace 落点、坏包闸、
# 卸载）。Scratch marker: /var/tmp/accept-m30。HOME 钉 /home（lib.drv）—
# adb 原生 HOME=/ 会把 CLI 面劈到 /.aginx 去开第二注册表（M30 实测）。
#
# Prereq: dup 已通过 agpkg 装好（M30c bring-up），新 aginx-carrier 已在
# /var/bin。Suites never install packages themselves — that's bring-up work.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m30

# --- dup 四件套产物 ---------------------------------------------------------

drv 'dup --version'
expect_rc 0 "dup face answers --version"
expect_out '^dup [0-9]' "dup --version prints version"

drv 'test -f /var/lib/agpkg/skills/dup/SKILL.md'
expect_rc 0 "dup SKILL.md landed in skill universe"

drv 'dup --help'
expect_rc 0 "dup --help rc 0"
expect_out 'commit|pull|push' "dup --help lists sync subcommands"

# --- dup 离线本地环（init → commit → log → status）-------------------------

drv "rm -rf ${M} && mkdir -p ${M}/ws && cd ${M}/ws && printf '# t\n' > profile.md"
expect_rc 0 "scratch workspace created"

drv "cd ${M}/ws && OPENCARRIER_URL=http://127.0.0.1:9 OPENCARRIER_API_KEY=dev-dummy dup init"
expect_rc 0 "dup init links remote (dummy creds, offline)"
drv "test -f ${M}/ws/.dup/state.json"
expect_rc 0 ".dup/state.json written"

drv "cd ${M}/ws && mkdir -p flows/demo && printf -- '---\nname: demo\ndescription: M30 验收流程\nversion: 0.1.0\n---\nbody\n' > flows/demo/flow.md && dup commit -m 'first snapshot'"
expect_rc 0 "dup commit snapshots tree"
expect_out '[0-9a-f]{8,}' "dup commit prints short id"

drv "cd ${M}/ws && dup log"
expect_rc 0 "dup log rc 0"
expect_out 'first snapshot' "dup log shows the message"

drv "cd ${M}/ws && dup status"
expect_rc 0 "dup status rc 0 (clean tree)"

# --- agent install --file ---------------------------------------------------

drv "mkdir -p ${M}/src/flows/demo && printf '# m30\n' > ${M}/src/profile.md && printf -- '---\nname: demo\ndescription: M30 验收流程\nversion: 0.1.0\nshell_allow:\n  - pattern: tar -tf *\n    match: [tar -tf x.tar]\n    not_match: [rm -rf /]\n---\nbody\n' > ${M}/src/flows/demo/flow.md && tar -cf ${M}/clone.tar -C ${M}/src profile.md flows"
expect_rc 0 "test clone tar built on device"

drv "ag agent install accept-m30 --file ${M}/clone.tar --dry-run"
expect_rc 0 "install --dry-run passes format gate"
expect_out '预检通过' "dry-run says 预检通过"
expect_out '未安装' "dry-run did not install"

drv "test ! -e /home/.aginx/carrier/workspaces/accept-m30"
expect_rc 0 "dry-run left no workspace behind"

drv "ag agent install accept-m30 --file ${M}/clone.tar"
expect_rc 0 "install --file installs"
expect_out '已安装' "install says 已安装"

drv "test -f /home/.aginx/carrier/workspaces/accept-m30/profile.md"
expect_rc 0 "workspace landed under /home/.aginx/carrier/workspaces"

drv 'ag agent list --json'
expect_rc 0 "ag agent list --json rc 0"
expect_py 'envelope lists accept-m30' 'j["ok"] is True and any(r["id"] == "accept-m30" for r in j["data"])'

drv "mkdir -p ${M}/bad/flows/x && printf -- '---\nname: x\nversion: 1\n---\nb\n' > ${M}/bad/flows/x/flow.md && tar -cf ${M}/bad.tar -C ${M}/bad flows"
expect_rc 0 "bad clone tar built"

drv "ag agent install accept-m30 --file ${M}/bad.tar --dry-run"
if [ "${DRV_RC}" = "1" ]; then
  t_ok "bad tar dry-run rejected (rc 1)"
else
  t_fail "bad tar dry-run rejected (want rc 1)"
fi
expect_out '预检未通过' "bad tar names the gate failure"

drv "ag agent remove accept-m30"
expect_rc 0 "agent remove rc 0"
drv "test ! -e /home/.aginx/carrier/workspaces/accept-m30"
expect_rc 0 "workspace gone after remove"
drv 'ag agent list --json'
expect_py 'envelope no longer lists accept-m30' 'not any(r["id"] == "accept-m30" for r in j["data"])'

# --- cleanup ----------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch marker cleaned"

suite_done
