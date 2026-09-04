#!/usr/bin/env bash
# m42b acceptance — QR 眼分支（#139）。
# fixture 是设备实拍（MacBook Retina 满屏窗、~20cm）：真实摩尔纹/镜头模糊
# 里的解码回归。光学全流程（拍照→join）收据在 docs/HARDWARE.md，不在本套
# （需要人持机对屏，不适合无人值守验收）。
set -euo pipefail
. "$(dirname "$0")/lib.sh"
suite_require_device

SCRATCH=/var/tmp/accept-m42b
FIXDIR="$(cd "$(dirname "$0")/../.." && pwd)/crates/agqr/fixtures"

cleanup() {
  drv "rm -rf $SCRATCH"
}
trap cleanup EXIT

drv "rm -rf $SCRATCH && mkdir -p $SCRATCH"
adbx push "$FIXDIR/screen-wifi-dummy.jpg" "$SCRATCH/qr.jpg" >/dev/null
adbx push "$FIXDIR/plain-gray.jpg" "$SCRATCH/noqr.jpg" >/dev/null

# --- 1. 实拍码：光学链回归（DCT 阶梯 + Bradley 兜底全在里面） ---------------

drv "/usr/bin/agqr $SCRATCH/qr.jpg"
expect_rc 0 '实拍 fixture 解出（rc）'
expect_out 'P:1234567890;;$' 'payload 原文（DUMMY，无真实凭据）'

# --- 2. 无码/错误语义 ---------------------------------------------------------

drv "/usr/bin/agqr $SCRATCH/noqr.jpg"
expect_rc 1 '无码图 rc=1（非错误）'

drv "/usr/bin/agqr $SCRATCH/missing.jpg"
expect_rc 2 '读图失败 rc=2'

# --- 3. 路由 shim -------------------------------------------------------------

drv "ag-qr $SCRATCH/qr.jpg"
expect_rc 0 'ag-qr shim 转发解出'

# 不跑全局 `ag commands --check`：设备 /var/bin 语音件（ag-tts 等）本就无
# .agmd——四件套化是 bake 后的事；M42b 只对自己引入的 shim 负责，上面的
# 功能断言即覆盖。

suite_done
