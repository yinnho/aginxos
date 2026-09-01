# OpenLogi 对照研究（2026-09-01，备用参考）

调研对象：[AprilNEA/OpenLogi](https://github.com/AprilNEA/OpenLogi)
（18.4k★，Apache-2.0，纯 Rust，活跃维护中）——罗技官方驱动
Options+ 的开源替代：无账号/无遥测/local-first，通过 HID++ 协议
直控罗技外设（按键重映射/DPI/SmartShift/手势/Litra 灯/UVC 摄像头
参数）。跨 macOS/Linux/Windows，Linux 一等公民，GUI 用 GPUI。

## 对我们是什么

它不是同类产品，是**「Rust 写到 ioctl 为止」这条立法（见
[DEVICE.md](DEVICE.md) 语言分层节）的 18k 星生产级实证**：一个
daemon + CLI + IPC + 硬件访问层的完整 Rust 外设栈，形状与我们
aginx/carrier/agc/periph 天然同构。动 `periph` crate 前的必读参考。

## crate 切法 → periph 对照

| OpenLogi | 干什么 | 对应我们 |
|---|---|---|
| `openlogi-hid` | hidraw/平台 API 封装（transport/permissions/probe_cache/host） | periph 的 /dev 访问层形状 |
| `openlogi-hidpp` + `hidpp-derive` | HID++ 协议层 + 宏派生 | vendor 协议薄绑定范本（RKNN/MPP 绑定可仿） |
| `openlogi-camera` | UVC 控制 + 采集（linux/windows/macos 分文件 + uvc 核心） | **eye.rs 现成参考** |
| `openlogi-device-registry` | 设备识别/能力表 | periph 设备注册表思路 |
| `openlogi-agent` | 常驻 daemon（lifecycle/pairing/takeover/tray/shutdown） | carrier 同构 |
| `openlogi-cli` + `openlogi-ipc` | CLI + daemon 间 IPC | agc ↔ carrier 通道同构 |
| `openlogi-hook` / `inject` | OS 输入钩子/注入 | 我们不需要（卡上无人输入面） |

## 动 periph 时具体可白嫖的点

1. **`openlogi-hid` 的 Linux 侧**：hidraw 打开/读写封装、**权限
   处理**（permissions.rs——udev/ACL 场景我们同样会遇到）、设备
   probe 缓存、热插拔监听。periph 的 v4l2/pcm 设备枚举同构
2. **`openlogi-camera` 的平台分文件模式**：`capture_linux.rs` /
   `uvc_linux.rs` 与平台核心分离——periph 五官模块照此切
   （`eye_linux.rs` 等），Termux 阶段 stub 才有干净挂点
3. **`hidpp-derive` 宏派生协议消息**：如果 RKNN 绑定的张量描述/
   MPP 的 buffer 声明写得烦了，这是「协议层声明式化」的成熟样式
4. **`openlogi-agent` 的 lifecycle/shutdown/takeover**：daemon
   优雅退出与新旧实例接管——carrier 长驻进程管理的参照

## 一句话

内核 C、协议 FFI 包薄、访问层全 Rust、daemon+CLI+IPC 分层——
OpenLogi 用三年生产验证了这套切法；periph 立项时按它的形状起手，
省一轮自己摸。
