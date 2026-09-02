#!/usr/bin/env bash
# M31 acceptance — agb 四件套 + ag-browser/ag-web 路由面（D3 批1 收口）。
#
# Covers: agb 安装产物（/var/bin 面 + SKILL.md skill 宇宙）、机读信封面
# （`agb tool <name>` D1 envelope）、`ag browser --help` 拦截（目标不执行）、
# 路由面真抓取（web fetch / browser navigate）、组表注册（web 组）、
# `ag commands --check` 全绿。桥的完整 LLM 回路（agent 调 web_fetch →
# agb_bridge spawn → 信封 → 结果回填）是 bring-up 级验证，收据在
# docs/HARDWARE.md M31 条目，不进套件（LLM 轮太慢且依赖外部 brain）。
#
# Prereq: agb 已通过 agpkg 装好、新 aginx-carrier 已在 /var/bin（M31c
# bring-up）。Suites never install packages themselves。Scratch marker:
# /var/tmp/accept-m31。网络面用 example.com（稳定、无风控）；引擎面走
# 本机 :8089。
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m31
drv "rm -rf ${M} && mkdir -p ${M}"
expect_rc 0 "scratch dir created"

# Legrand AP 上直连 HTTPS 偶发失败（无 v6 路由 + 路由器 DNS 抖动，
# HARDWARE.md M31 观察记录）。agb 每次调用都是新进程、无 DNS 缓存，所以
# 网络面检查带一次重试——套件验的是 agb 面，不是 AP 的稳定性。
drv_net() {
  local cmd="$1" i
  for i in 1 2 3; do
    drv "${cmd}"
    [ "${DRV_RC}" = "0" ] && return 0
    sleep 3
  done
  return 1
}

# --- agb 四件套产物 ---------------------------------------------------------

drv 'test -x /var/bin/agb'
expect_rc 0 "agb binary is executable in /var/bin"

drv 'test -f /var/lib/agpkg/skills/agb/SKILL.md'
expect_rc 0 "agb SKILL.md landed in skill universe"

drv 'agb --version'
expect_rc 0 "agb --version rc 0"
expect_out '^agb [0-9]' "agb --version prints version"

# --- 机读面：D1 信封 --------------------------------------------------------

drv 'echo "{}" | agb tool browser_close'
expect_rc 0 "agb tool browser_close rc 0"
expect_out '"ok": ?true' "envelope carries ok:true"
expect_out 'stateless' "envelope data mentions stateless no-op"

drv "echo '{\"url\": \"https://x.io/p?api_key=sk-1\"}' | agb tool web_fetch > ${M}/taint.out 2>&1; echo rc=\$?; cat ${M}/taint.out"
expect_out 'Taint violation' "taint gate fires on keyed URL"
drv "grep -q 'Taint violation' ${M}/taint.out && grep -qE '\"ok\": ?false' ${M}/taint.out"
expect_rc 0 "taint failure is a clean envelope, not a crash"

drv 'echo "{}" | agb tool nope_such_tool'
expect_rc 1 "unknown tool name exits nonzero"
expect_out '"ok": ?false' "unknown tool reports envelope error"

# --- 路由面：ag browser / ag web（M24 ag 路由器最长前缀派发）-----------------

drv 'ag browser --help'
expect_rc 0 "ag browser --help intercepted rc 0"
expect_out 'ag:summary|browser automation' "help prints the shim summary"
expect_out 'navigate <url>' "help prints usage args"

# 拦截=目标绝不执行：--help 下 agb 不该跑任何工具（browser_close 是无副作用
# 的 no-op，这里验证的是 help 输出里没有工具执行痕迹——信封/Title 都不该出现）。
drv "ag browser --help 2>&1 | grep -qE '\"ok\":|Title:'"
expect_rc 1 "ag browser --help executed nothing (no envelope/Title in output)"

drv_net 'ag web fetch https://example.com'
expect_rc 0 "ag web fetch rc 0"
expect_out 'HTTP 200' "fetch reports 200"
expect_out 'Example Domain' "fetch returns real page content"
expect_out 'EXTCONTENT' "external content wrap is present"

drv_net 'ag browser navigate https://example.com'
expect_rc 0 "ag browser navigate rc 0"
expect_out 'Title: Example Domain' "navigate returns engine page title"

drv_net 'ag web search rust --max-results 2'
expect_rc 0 "ag web search rc 0"
expect_out 'https?://' "search returns result links"

# --- 组表 + 路由门禁 --------------------------------------------------------

drv 'grep -q "^web=" /etc/ag/groups.desc'
expect_rc 0 "web group registered in groups.desc"

drv 'ag commands --check'
expect_rc 0 "ag commands --check green"
expect_out 'OK$|commands OK' "check reports all commands OK"

drv 'ag commands --json'
expect_rc 0 "ag commands --json rc 0"
expect_out '"(browser|web)"' "router registry lists browser/web"

# --- 收尾 -------------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch cleaned"

suite_done
