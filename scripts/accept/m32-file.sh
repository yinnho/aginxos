#!/usr/bin/env bash
# M32 acceptance — agf 四件套 + ag-file 路由面（D3 批2 收口）。
#
# Covers: agf 安装产物（/var/bin 面 + SKILL.md skill 宇宙）、机读信封面
# （`agf tool <name>` D1 envelope）、二进制拒绝/目录纠偏 steer、人面
# write/read 回路、`ag file --help` 拦截（目标不执行）、路由面真读写、
# 组表注册（files 组）、`ag commands --check` 全绿。桥的完整 LLM 回路
# （agent 调 file_write → agf_bridge spawn → 信封 → 结果回填）是 bring-up
# 级验证，收据在 docs/HARDWARE.md M32 条目，不进套件（LLM 轮太慢且依赖
# 外部 brain）。
#
# Prereq: agf 已通过 agpkg 装好、新 aginx-carrier 已在 /var/bin（M32c
# bring-up）。Suites never install packages themselves。Scratch marker:
# /var/tmp/accept-m32。全程本机文件面，无网络依赖。
set -euo pipefail
. "$(dirname "$0")/lib.sh"

suite_require_device

M=/var/tmp/accept-m32
drv "rm -rf ${M} && mkdir -p ${M}"
expect_rc 0 "scratch dir created"

# --- agf 四件套产物 ---------------------------------------------------------

drv 'test -x /var/bin/agf'
expect_rc 0 "agf binary is executable in /var/bin"

drv 'head -c 4 /var/bin/agf | od -An -tx1 | grep -q "7f 45 4c 46"'
expect_rc 0 "agf is a real ELF binary (not a raw tar/gzip copy)"

drv 'test -f /var/lib/agpkg/skills/agf/SKILL.md'
expect_rc 0 "agf SKILL.md landed in skill universe"

drv 'agf --version'
expect_rc 0 "agf --version rc 0"
expect_out '^agf [0-9]' "agf --version prints version"

# --- 机读面：D1 信封 --------------------------------------------------------

drv 'echo "{\"path\":\"/etc/hostname\"}" | agf tool file_read'
expect_rc 0 "agf tool file_read rc 0"
expect_out '"ok": ?true' "envelope carries ok:true"
expect_out 'aginxos' "envelope data carries file content"

drv 'echo "{}" | agf tool nope_such_tool'
expect_rc 1 "unknown tool name exits nonzero"
expect_out '"ok": ?false' "unknown tool reports envelope error"

# 二进制拒绝 steer：PNG 魔数 → 干净信封 + 指路 image_analyze。
drv "printf '\\x89PNG\\r\\n\\x1a\\nJUNK' > ${M}/fake.png && echo \"{\\\"path\\\":\\\"${M}/fake.png\\\"}\" | agf tool file_read > ${M}/steer.out 2>&1; echo rc=\$?; cat ${M}/steer.out"
expect_out 'image_analyze' "binary reject steers to image_analyze"
drv "grep -q 'image_analyze' ${M}/steer.out && grep -qE '\"ok\": ?false' ${M}/steer.out"
expect_rc 0 "binary reject is a clean envelope, not garbage bytes"

drv "echo \"{\\\"path\\\":\\\"${M}/fake.png\\\"}\" | agf tool image_analyze"
expect_rc 0 "image_analyze handles the PNG envelope rc 0"
# data 是信封内再转义的 JSON 串：断言要按 \" 转义形态匹配。
expect_out '\\\"format\\\": ?\\\"png\\\"' "image_analyze reports png format"

# --- 人面：write/read/ls 回路 + 纠偏 ---------------------------------------

drv 'agf write /var/tmp/accept-m32/roundtrip.md --content "m32 accept roundtrip"'
expect_rc 0 "agf write rc 0"

drv 'agf read /var/tmp/accept-m32/roundtrip.md'
expect_rc 0 "agf read back rc 0"
expect_out 'm32 accept roundtrip' "read returns what write put"

drv 'agf ls /var/tmp/accept-m32/roundtrip.md 2>&1; echo "rc=$?" > /dev/null'
expect_out 'file_read' "ls on a file steers to file_read (anti tool-loop)"

drv 'agf ls /var/tmp/accept-m32'
expect_rc 0 "agf ls on a dir rc 0"
expect_out 'roundtrip' "ls lists the written file"

# --- 路由面：ag file（M24 ag 路由器最长前缀派发）---------------------------

drv 'ag file --help'
expect_rc 0 "ag file --help intercepted rc 0"
expect_out 'ag:summary|file read/write/list/convert' "help prints the shim summary"
expect_out 'read <path>' "help prints usage args"

# 拦截=目标绝不执行：help 输出里不该有信封/工具执行痕迹。
drv "ag file --help 2>&1 | grep -qE '\"ok\":|AGF_'"
expect_rc 1 "ag file --help executed nothing (no envelope in output)"

drv 'ag file read /etc/hostname'
expect_rc 0 "ag file read rc 0"
expect_out 'aginxos' "route-through read returns real content"

drv 'ag file write /var/tmp/accept-m32/via-ag.md --content "via router"'
expect_rc 0 "ag file write rc 0"
drv 'ag file read /var/tmp/accept-m32/via-ag.md'
expect_out 'via router' "router-written file reads back"

# --- 组表 + 路由门禁 --------------------------------------------------------

drv 'grep -q "^files=" /etc/ag/groups.desc'
expect_rc 0 "files group registered in groups.desc"

drv 'ag commands --check'
expect_rc 0 "ag commands --check green"
expect_out 'OK$|commands OK' "check reports all commands OK"

drv 'ag commands --json'
expect_rc 0 "ag commands --json rc 0"
expect_out 'ag-file' "router registry lists ag-file"

# --- 收尾 -------------------------------------------------------------------

drv "rm -rf ${M}"
expect_rc 0 "scratch cleaned"

suite_done
