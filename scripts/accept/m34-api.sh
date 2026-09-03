#!/usr/bin/env bash
# M34 acceptance — api 面 + 声明式工具链（M34 a/b/c）。
#
# Covers: ag api --help 拦截（目标不执行）、api list 人面/信封面、机读 call
# （stdin JSON+_ctx → D1 信封，extract 出城市非空）、人面 call、raw 直通、
# 未知工具错误面（api_tool_unknown 信封）、--param k=v 缺等号闸、注册幂等
# （同名重注册 [[tool]] 块数不增）、cron 一跳（scratch 调度 → 轮询 api cron
# --json 到 fired → sqlite 行 python3 验证）→ 清场。
#
# 守护侧 30s 委托节拍（start.rs tick → spawn api cron）和 LLM 回路（acp 轮
# 调 ip_city 过桥 spawn）依赖外部 brain 与守护节奏，收据在 docs/HARDWARE.md
# M34 条目——套件不依赖（同 m33 惯例）。全局 ~/.aginx/carrier/api_tools.toml
# 只读不动；全部注册走 /var/tmp/accept-m34 工作区 + 显式 --toml。
#
# Prereq: M34 aginx-carrier 在 /var/bin、ag-api shim 在 /usr/bin、api 组在
# /etc/ag/groups.desc。Scratch marker: /var/tmp/accept-m34。网络依赖:
# ip-api.com（明文 HTTP——免费档 HTTPS 403，见 HARDWARE.md M33）。
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m34
# 工件在 host 起草（套件可重入），push 进设备 scratch——设备侧不现造文件。
H=$(mktemp -d)
trap 'rm -rf "${H}"' EXIT
drv "rm -rf ${M} && mkdir -p ${M}/ws"
expect_rc 0 "scratch dir created"

# --- 路由面 -------------------------------------------------------------------

drv 'ag api --help'
expect_rc 0 "ag api --help intercepted rc 0"
expect_out 'declarative API tool face' "help prints the shim summary"

drv "ag api --help 2>&1 | grep -qE '\"ok\":'"
expect_rc 1 "ag api --help executed nothing (no envelope in output)"

drv 'test -x /usr/bin/ag-api'
expect_rc 0 "ag-api shim is executable in /usr/bin"

drv 'grep -q "^api=" /etc/ag/groups.desc'
expect_rc 0 "api group registered in groups.desc"

# --- 注册（scratch 工作区，不碰全局）------------------------------------------

cat > "${H}/tool.toml" <<'TOML'
[[tool]]
name = "m34_probe"
description = "acceptance probe tool"
url = "http://ip-api.com/json/"
method = "GET"
[tool.error_check]
field = "status"
expect = "success"
[tool.extract]
city = { path = "city" }
ip = { path = "query" }
TOML
adbx push "${H}/tool.toml" "${M}/ws/tool.toml" >/dev/null

drv "aginx-carrier api register --workspace ${M}/ws --file ${M}/ws/tool.toml"
expect_rc 0 "register m34_probe to scratch workspace"
expect_out 'm34_probe' "register reports the tool name"

# 幂等：同名重注册块数不增（写手单源在 CLI）。
drv "aginx-carrier api register --workspace ${M}/ws --file ${M}/ws/tool.toml >/dev/null && grep -c '\[\[tool\]\]' ${M}/ws/api_tools.toml"
expect_rc 0 "re-register exits 0"
expect_out '^1$' "same-name re-register keeps exactly one block"

# --- list ---------------------------------------------------------------------

drv "aginx-carrier api list --toml ${M}/ws/api_tools.toml"
expect_rc 0 "api list rc 0"
expect_out 'm34_probe' "list shows the scratch tool"

drv "aginx-carrier api list --toml ${M}/ws/api_tools.toml --json"
expect_rc 0 "api list --json rc 0"
expect_py 'api list --json is a D1 envelope' 'j.get("ok") is True and j["meta"]["count"] == 1 and j["data"][0]["name"] == "m34_probe"'

# --- call：机读面（stdin JSON + _ctx）-----------------------------------------

drv "echo '{\"_ctx\":{\"sender_id\":\"accept-m34\",\"channel_type\":\"cli\"}}' | aginx-carrier api call m34_probe --json --toml ${M}/ws/api_tools.toml"
expect_rc 0 "api call machine face rc 0"
expect_py 'call envelope ok with extracted city' 'j.get("ok") is True and "city" in j["data"]'

# --- call：人面（--param）------------------------------------------------------

drv "aginx-carrier api call m34_probe --toml ${M}/ws/api_tools.toml"
expect_rc 0 "api call human face rc 0"
expect_out 'city' "human face prints extracted JSON"

# --- call：未知工具错误面 --------------------------------------------------------

drv "echo '{}' | aginx-carrier api call nope_such --json --toml ${M}/ws/api_tools.toml"
expect_rc 1 "unknown api tool exits nonzero"
expect_py 'unknown tool reports api_tool_unknown envelope' 'j.get("ok") is False and j["error"]["code"] == "api_tool_unknown"'

# --- --param 语法闸 --------------------------------------------------------------

drv "aginx-carrier api call m34_probe --param noequals --toml ${M}/ws/api_tools.toml --json"
expect_rc 1 "malformed --param (no =) exits nonzero"
expect_py 'malformed param reports api_bad_input' 'j.get("ok") is False and j["error"]["code"] == "api_bad_input"'

# --- raw 直通 -------------------------------------------------------------------

drv 'aginx-carrier api raw GET "http://ip-api.com/json/?fields=status"'
expect_rc 0 "api raw GET rc 0"
expect_out 'success' "raw passthrough returns body"

drv 'aginx-carrier api raw PATCH "http://ip-api.com/json/"'
expect_rc 1 "api raw rejects unsupported method"

# --- cron 一跳（到点发射 + sqlite 落库）------------------------------------------

cat > "${H}/cron.toml" <<'TOML'
[[tool]]
name = "m34_cron"
description = "acceptance cron probe"
url = "http://ip-api.com/json/"
method = "GET"
[tool.error_check]
field = "status"
expect = "success"
[tool.extract]
city = { path = "city" }
[tool.cron]
schedule = "* * * * *"
save_to = "sqlite:m34-cron.db"
table = "m34_cron_rows"
TOML
adbx push "${H}/cron.toml" "${M}/ws/cron.toml" >/dev/null

drv "aginx-carrier api register --workspace ${M}/ws --file ${M}/ws/cron.toml"
expect_rc 0 "register m34_cron with [tool.cron]"
drv "grep -c 'tool.cron' ${M}/ws/api_tools.toml"
expect_out '^1$' "registered file keeps the cron section"

# 分钟粒度调度 × 秒<30 才发：轮询最坏一个整分钟。
FIRED=0
for i in $(seq 1 14); do
  drv "aginx-carrier api cron --json --toml ${M}/ws/api_tools.toml --home ${M}/ws 2>/dev/null || true"
  if printf '%s\n' "${DRV_OUT}" | grep -q '"fired":\[{'; then
    FIRED=1
    break
  fi
  sleep 5
done
if [ "${FIRED}" = "1" ]; then
  t_ok "api cron fired the due tool (second<30 window)"
else
  t_fail "api cron fired the due tool (second<30 window)"
fi

drv "/var/bin/python3 -c \"import sqlite3;c=sqlite3.connect('${M}/ws/m34-cron.db');r=c.execute('select tool_name,raw_response from m34_cron_rows order by id desc limit 1').fetchall();print(r[0][0] if r else 'EMPTY')\""
expect_rc 0 "cron row readable via device python3"
expect_out '^m34_cron$' "sqlite row holds tool_name m34_cron"

# --- 收尾 -------------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch cleaned"

suite_done
