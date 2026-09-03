#!/usr/bin/env bash
# M33 acceptance — carrier tool/sys 面 + 内核耦合工具桥（D3 批3 收口）。
#
# Covers: 机读面 D1 信封（system_time / tool_unknown / cron_create 缺身份
# 错误面）、schedule_* kv 回路（create→list→delete→空）、agent_list、
# 人面（ag agent list / ag cron list / ag sys time --json）、ag sys time
# --help 拦截（目标不执行）、四个 M33 shim 在位、sys 组注册、
# ag commands --check 全绿。
#
# Bring-up 级验证不进套件（同 M32 惯例）：cron DB-as-bus reconcile 收条
# （CLI 落 one_shot 任务 → 守护 15s tick 收养 → 到点发射 → 自动删除）和
# LLM 回路（me 调 agent_send → 桥 spawn 子进程 → clone-creator 真轮 →
# 原话回传）依赖外部 brain + 守护节奏，收据在 docs/HARDWARE.md M33 条目。
# 套件不建 cron 任务（避免在守护上留下真实发射副作用），只走错误面。
#
# Prereq: 新 aginx-carrier 已在 /var/bin、四个 shim 已在 /usr/bin（M33c
# bring-up）。Scratch marker: /var/tmp/accept-m33。无网络依赖。
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m33
drv "rm -rf ${M} && mkdir -p ${M}"
expect_rc 0 "scratch dir created"

# --- shim 在位 + 路由面 ------------------------------------------------------

drv 'test -x /usr/bin/ag-cron -a -x /usr/bin/ag-agent -a -x /usr/bin/ag-sys-time -a -x /usr/bin/ag-sys-location'
expect_rc 0 "four M33 shims are executable in /usr/bin"

drv 'ag sys time --help'
expect_rc 0 "ag sys time --help intercepted rc 0"
expect_out 'current date/time' "help prints the shim summary"

# 拦截=目标绝不执行：help 输出里不该有信封/工具执行痕迹。
drv "ag sys time --help 2>&1 | grep -qE '\"ok\":'"
expect_rc 1 "ag sys time --help executed nothing (no envelope in output)"

drv 'ag sys time --json'
expect_rc 0 "ag sys time --json rc 0"
expect_py 'ag sys time --json is a D1 envelope with time payload' 'j.get("ok") is True and "unix_epoch" in j["data"]'

drv 'ag agent list'
expect_rc 0 "ag agent list rc 0"
expect_out 'me' "agent list shows the system identity agent"

drv 'ag cron list'
expect_rc 0 "ag cron list rc 0"
expect_py 'ag cron list is a D1 envelope' 'j.get("ok") is True and "data" in j'

# --- 机读面：D1 信封（不 boot kernel 的两个）-------------------------------

drv 'echo "{}" | aginx-carrier tool system_time'
expect_rc 0 "tool system_time rc 0"
expect_py 'system_time envelope ok with epoch' 'j.get("ok") is True and "unix_epoch" in j["data"]'

drv 'echo "{}" | aginx-carrier tool nope_such'
expect_rc 1 "unknown tool name exits nonzero"
expect_py 'unknown tool reports tool_unknown envelope' 'j.get("ok") is False and j["error"]["code"] == "tool_unknown"'

# --- 机读面：内核工具错误面（boot kernel 但无副作用）------------------------

drv 'echo "{}" | aginx-carrier tool cron_create'
expect_rc 1 "cron_create without caller identity exits nonzero"
expect_py 'cron_create error envelope mentions agent id' 'j.get("ok") is False and "Agent ID required" in j["error"]["message"]'

# --- 机读面：schedule kv 回路（create→list→delete→空）-----------------------

drv 'echo "{\"_ctx\":{\"caller_agent_id\":\"me\"},\"description\":\"m33 accept\",\"schedule\":\"every 5 minutes\"}" | aginx-carrier tool schedule_create'
expect_rc 0 "schedule_create rc 0"
expect_out 'Schedule created' "schedule_create reports created"

SCHED_ID="$(printf '%s' "${DRV_OUT}" | grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' | head -1 || true)"
if [ -n "${SCHED_ID}" ]; then
  t_ok "schedule_create returned an id (${SCHED_ID:0:8}…)"
else
  t_fail "schedule_create returned an id"
fi

drv 'echo "{\"_ctx\":{\"caller_agent_id\":\"me\"}}" | aginx-carrier tool schedule_list'
expect_rc 0 "schedule_list rc 0"
expect_out 'm33 accept' "schedule_list shows the created schedule"

drv "echo \"{\\\"_ctx\\\":{\\\"caller_agent_id\\\":\\\"me\\\"},\\\"id\\\":\\\"${SCHED_ID}\\\"}\" | aginx-carrier tool schedule_delete"
expect_rc 0 "schedule_delete rc 0"
expect_out 'deleted' "schedule_delete reports deleted"

drv 'echo "{\"_ctx\":{\"caller_agent_id\":\"me\"}}" | aginx-carrier tool schedule_list'
expect_rc 0 "schedule_list after delete rc 0"
expect_out 'No scheduled tasks' "schedule_list is empty again"

# --- 机读面：agent_list（boot kernel）---------------------------------------

drv 'echo "{\"_ctx\":{\"caller_agent_id\":\"me\"}}" | aginx-carrier tool agent_list'
expect_rc 0 "tool agent_list rc 0"
expect_out 'me' "tool agent_list shows registered agents"

# --- 人面新子命令：agent send/kill 语法闸 ------------------------------------

drv 'ag agent send 2>&1; true'
expect_out 'usage|Usage|--message|missing|required' "ag agent send without args prints usage"

# --- 组表 + 路由门禁 ---------------------------------------------------------

drv 'grep -q "^sys=" /etc/ag/groups.desc'
expect_rc 0 "sys group registered in groups.desc"

drv 'ag commands --check'
expect_rc 0 "ag commands --check green"

drv 'ag commands --json'
expect_rc 0 "ag commands --json rc 0"
expect_out 'ag-sys-time' "router registry lists ag-sys-time"
expect_out 'ag-sys-location' "router registry lists ag-sys-location"

# --- 收尾 -------------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch cleaned"

suite_done
