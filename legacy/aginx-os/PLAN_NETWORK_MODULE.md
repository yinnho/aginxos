# Aginx OS 外部网络模块方案

Last updated: 2026-04-11

## 背景

Pixel 5 (Redfin) 使用 GKI (Generic Kernel Image)，ABL 强制验证内核签名，自定义 Linux 内核无法启动。
裸机 Rust 内核已成功在 Pixel 5 上执行（framebuffer 填色 + 文字显示）。

裸机方案的挑战：WiFi (QCA6390) 和 4G (X52 基带) 驱动开发量巨大。
解决方案：使用外部硬件模块处理网络，通过 USB Type-C 与 Pixel 5 通信。

## 硬件选型：合宙 Air8000

### 为什么选 Air8000

| 需求 | Air8000 支持 |
|------|-------------|
| 4G 联网 | Cat.1 全网通（移动/联通/电信），VoLTE |
| WiFi | 内置 WiFi，支持 4G→WiFi 转发 |
| USB 连接 | USB CDC 虚拟串口 + RNDIS/ECM USB 网卡 |
| 尺寸 | 22.3 × 22.4 × 2.3mm（可嵌入手机壳） |
| 价格 | 核心板约 ¥54 |
| 开发 | LuatOS（Lua 脚本），几十行即可配置网络转发 |
| 多网融合 | 4G/WiFi/以太网自动切换 |

### 不选其他方案的原因

- **ESP32-C3 + Air780E 组合**：需要两块板子，合宙 ESP32 已停产，需买其他品牌
- **Air780E 单模组**：只有 4G，没有 WiFi
- **Air780EPS**：低功耗 4G，但无 WiFi
- **Pixel 5 直连 WiFi/4G**：驱动开发量 2000+ 行，不现实

## 系统架构

```
┌─────────────────────────────────────────────┐
│            手机外壳 (3D 打印)                │
│                                             │
│  ┌───────────┐    USB Type-C    ┌─────────┐ │
│  │ Pixel 5   │◄───────────────►│ Air8000 │ │
│  │           │  CDC-ECM 网卡    │         │ │
│  │ Aginx OS  │                  │ 4G Cat.1├─┤── SIM 卡
│  │ (bare     │  192.168.42.2    │ WiFi    │ │
│  │  metal    │                  │ USB     │ │
│  │  Rust)    │◄──IP packets───►│         │ │
│  └───────────┘                  └─────────┘ │
│       │                              │      │
│   Type-C 充电              4G天线 + WiFi天线 │
└─────────────────────────────────────────────┘
```

### 数据流

```
Pixel 5 应用层 (TCP shell / Agent)
    ↓
smoltcp TCP/IP 栈
    ↓
USB CDC-ECM 虚拟网卡 (usb0)
    ↓
USB Type-C 物理连接
    ↓
Air8000 LuatOS 网络转发
    ↓
4G LTE / WiFi → 互联网
```

## 开发分工

### Pixel 5 端 (Rust bare-metal)

已有代码基础：
- `kernel/src/usb_dwc3.rs` — DWC3 USB gadget 骨架 (116 行)
- `kernel/src/usb_net.rs` — USB CDC-ECM 网络驱动 (197 行)
- `kernel/src/ip_stack.rs` — 完整 TCP/IP 栈 (1644 行)
- `kernel/src/net.rs` — VirtIO-Net 驱动（可参考）
- `kernel/src/fb.rs` — Framebuffer（已验证工作）

需要完成：
1. **DWC3 USB gadget 驱动** — 完成设备模式初始化、端点配置
2. **CDC-ECM 函数** — USB 以太网仿真，提供虚拟网卡接口
3. **网络桥接** — USB CDC-ECM ↔ smoltcp TCP/IP 栈
4. **redfin 完整启动** — 启用 shell、任务调度、MMU（目前只做 MMU+FB）

### Air8000 端 (LuatOS Lua 脚本)

预估代码量：约 50-100 行 Lua

功能：
1. 4G 拨号联网
2. USB CDC-ECM 网络共享（4G→USB 转发）
3. WiFi AP 模式（可选）
4. 状态指示（LED 闪烁表示网络状态）

LuatOS 参考文档：https://docs.openluat.com/air8000/

## 刷机流程（回顾）

Pixel 5 裸机内核刷机方法（已验证工作）：

```bash
# 编译
cargo build --target aarch64-redfin.json --release --features board-redfin

# 生成 flat binary
rust-objcopy --strip-all -O binary \
  target/aarch64-redfin/release/aginx-kernel \
  target/aarch64-redfin/release/aginx-kernel.bin

# LZ4 压缩
python3 -c "import lz4.frame; \
  raw=open('target/aarch64-redfin/release/aginx-kernel.bin','rb').read(); \
  c=lz4.frame.compress(raw,len(raw),store_size=False); \
  open('/tmp/aginx-kernel.lz4','wb').write(c)"

# 创建 boot.img (AOSP mkbootimg)
python3 /tmp/mkbootimg/mkbootimg/mkbootimg.py \
  --kernel /tmp/aginx-kernel.lz4 \
  --ramdisk /tmp/stock_ramdisk \
  --os_version 11.0.0 \
  --os_patch_level 2021-10 \
  --header_version 3 \
  --output /tmp/boot.img

# 刷入
fastboot flash boot_a /tmp/boot.img && fastboot reboot
```

工具位置：
- mkbootimg: `/tmp/mkbootimg/mkbootimg/mkbootimg.py`
- stock ramdisk: `/tmp/stock_ramdisk`
- factory images: `/tmp/pixel5-restore/redfin-rq3a.211001.001/`

## 开发路线图

### Phase A：DWC3 USB Gadget 驱动（优先）
- 完成 DWC3 设备模式初始化
- 实现 CDC-ECM 功能（虚拟以太网卡）
- 在 QEMU 上先用 virtio-net 验证网络栈

### Phase B：Air8000 固件
- 购买 Air8000 核心板
- 编写 LuatOS 脚本：4G 拨号 + USB 网络共享
- 在电脑上测试 Air8000 USB 网卡功能

### Phase C：集成测试
- Pixel 5 + Air8000 USB 连接
- 验证端到端 TCP 连接
- 运行 TCP shell，从电脑远程连接

### Phase D：外壳设计
- 3D 打印手机壳，集成 Air8000 + SIM 卡槽 + 天线
- USB Type-C 连接器设计

## 参考资料

- [Air8000 产品手册](https://docs.openluat.com/air8000/product/)
- [Air8000 LuatOS 开发文档](https://docs.openluat.com/air8000/)
- [Air8000 硬件手册](https://docs.openluat.com/air8000/product/file/Air8000硬件手册V1.5.pdf)
- Pixel 5 裸机启动记录：`/Users/mac8684/.claude/projects/-Users-mac8684-Documents-agentos/memory/pixel5_boot.md`
