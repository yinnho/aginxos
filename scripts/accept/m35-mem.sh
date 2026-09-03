#!/usr/bin/env bash
# M35 acceptance — agmem 四件套 + ag mem 路由面（记忆面 D3 收口）。
#
# Covers: agmem 安装产物（/var/bin 面 + SKILL.md skill 宇宙）、机读信封面
# （`agmem tool kv_get` D1 envelope + agent 身份闸）、kv 身份三元组隔离
# （同一键三种身份互不可见）、人面 set/get/list/del 回路、knowledge 面
# （scratch workspace 的 add/read 回路 + 凭证闸拒收 api_key 指路 kv_set）、
# `ag mem --help` 拦截（目标不执行）、路由面直读、组表注册（mem 组）、
# `ag commands --check` 全绿。桥的完整 LLM 回路（agent 调 kv_set/kv_get →
# agmem_bridge spawn → 信封 → 结果回填）是 bring-up 级验证，收据在
# docs/HARDWARE.md M35 条目，不进套件（LLM 轮太慢且依赖外部 brain）。
#
# Prereq: agmem 已通过 agpkg 装好、新 aginx-carrier 已在 /var/bin（M35d
# bring-up）。Suites never install packages themselves。Scratch marker:
# /var/tmp/accept-m35。kv 断言全部用 m35acc. 前缀键 + 86bus 探针身份，
# 不碰真实化身域；数据库写入走真实 substrate（kv 面没有 scratch 隔离层，
# del 收尾清理）。
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m35
drv "rm -rf ${M} && mkdir -p ${M}/ws"
expect_rc 0 "scratch dir created"

# --- agmem 四件套产物 -------------------------------------------------------

drv 'test -x /var/bin/agmem'
expect_rc 0 "agmem binary is executable in /var/bin"

drv 'head -c 4 /var/bin/agmem | od -An -tx1 | grep -q "7f 45 4c 46"'
expect_rc 0 "agmem is a real ELF binary (not a raw tar/gzip copy)"

drv 'test -f /var/lib/agpkg/skills/agmem/SKILL.md'
expect_rc 0 "agmem SKILL.md landed in skill universe"

drv 'agmem --version'
expect_rc 0 "agmem --version rc 0"
expect_out '^agmem [0-9]' "agmem --version prints version"

# --- 机读面：D1 信封 + 身份语义 ----------------------------------------------
#
# agent 身份闸在桥侧（runtime），CLI 机读面按契约回落人面身份
# （me/default/local）——缺席 _ctx 的裸调用写进人域，不能崩。

drv 'echo "{\"key\":\"m35acc.probe\"}" | agmem tool kv_set'
expect_rc 1 "kv_set missing value param exits nonzero"
expect_out '"ok": ?false' "missing value reports envelope error"

drv 'printf %s "{\"key\":\"m35acc.probe\",\"value\":\"M35_ACC\",\"_ctx\":{\"agent_id\":\"86bus\",\"owner_id\":null,\"user_id\":null}}" | agmem tool kv_set'
expect_rc 0 "kv_set with explicit-null owner/user rc 0"
expect_out '"ok": ?true' "kv_set envelope carries ok:true"

drv 'printf %s "{\"key\":\"m35acc.probe\",\"_ctx\":{\"agent_id\":\"86bus\",\"owner_id\":null,\"user_id\":null}}" | agmem tool kv_get'
expect_rc 0 "kv_get same triple rc 0"
expect_out 'M35_ACC' "kv_get returns the written value"

# 身份隔离：人面默认身份（me/default/local）看不见 86bus 域的键。
drv 'printf %s "{\"key\":\"m35acc.probe\",\"_ctx\":{\"agent_id\":\"me\",\"owner_id\":\"default\",\"user_id\":\"local\"}}" | agmem tool kv_get'
expect_rc 0 "kv_get as human identity rc 0"
expect_out 'No value found' "human identity cannot see agent-domain key"

drv 'echo "{}" | agmem tool nope_such_tool'
expect_rc 1 "unknown tool name exits nonzero"
expect_out '"ok": ?false' "unknown tool reports envelope error"

# --- 人面：kv 回路 ----------------------------------------------------------

drv "agmem set m35acc.human '\"hello-m35\"'"
expect_rc 0 "human face kv set rc 0"

drv 'agmem get m35acc.human'
expect_rc 0 "human face kv get rc 0"
expect_out 'hello-m35' "human face reads back the value"

drv 'agmem list m35acc.'
expect_rc 0 "human face kv list rc 0"
expect_out 'm35acc.human' "list shows the prefixed key"

drv 'agmem del m35acc.human'
expect_rc 0 "human face kv del rc 0"
drv 'agmem get m35acc.human'
expect_out 'No value found' "deleted key is gone"

# --- knowledge 面：scratch workspace + 凭证闸 ------------------------------

drv "printf '%s' '# 测试条目正文' | agmem --workspace ${M}/ws k add m35验收"
expect_rc 0 "knowledge add in scratch workspace rc 0"

drv "agmem --workspace ${M}/ws k ls"
expect_rc 0 "knowledge list rc 0"
expect_out 'm35' "knowledge list shows the new file"

drv "printf '%s' 'api_key = sk-live-1234567890abcdef' | agmem --workspace ${M}/ws k add 泄密条目 > ${M}/cred.out 2>&1; echo rc=\$?; cat ${M}/cred.out"
expect_rc 0 "credential-shaped add runs to a reportable state"
expect_out 'kv_set' "credential gate steers to kv_set"
drv "grep -qE '\"ok\": ?false|Use kv_set' ${M}/cred.out"
expect_rc 0 "credential reject is a clean error, not garbage"

# --- 路由面：ag mem ---------------------------------------------------------

drv 'ag mem --help'
expect_rc 0 "ag mem --help intercepted rc 0"
expect_out '^ag mem ' "help prints router summary line"
expect_out 'usage: ag mem' "help prints usage"
# 拦截 = 目标不执行：usage 行不该来自 agmem 自身（agmem --help 无 usage: 前缀行）。
drv 'ag mem --help | grep -c "path: /usr/bin/ag-mem"'
expect_rc 0 "help names the shim path (router served it)"
expect_out '^1$' "exactly one path line"

drv 'ag mem get m35acc.router'
expect_rc 0 "ag mem get routes through the shim rc 0"
expect_out 'No value found' "unknown key via router reports cleanly"

drv "ag mem set m35acc.router '\"via-router\"' && ag mem get m35acc.router"
expect_rc 0 "ag mem set+get round trip via router rc 0"
expect_out 'via-router' "value round-trips through the router face"

drv 'ag mem del m35acc.router && ag mem del m35acc.probe'
expect_rc 0 "cleanup kv probe keys"

# --- 组表 + 门禁 -------------------------------------------------------------

drv 'grep -q "^mem=" /etc/ag/groups.desc'
expect_rc 0 "mem group registered in groups.desc"

drv 'ag commands --check'
expect_rc 0 "ag commands --check rc 0"
expect_out '[0-9]+ commands OK' "metadata gate green"

# --- 清理 --------------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch dir removed"

suite_done
