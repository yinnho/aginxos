# Aginx 开发计划

## 项目概述

**Aginx** = Agent OS for Pixel 5

一个基于 Rust 的独立操作系统，刷机运行于 Google Pixel 5 (redfin)，提供纯 CLI 环境，支持 WiFi 联网、SSH 访问和 Agent 运行时。

```
┌─────────────────────────────────────────────────────────┐
│                    aginx runtime                         │
│            (Agent 调度 + LLM 调用 + 工具)                 │
├─────────────────────────────────────────────────────────┤
│                    SSH Server                            │
│                  (远程交互入口)                           │
├─────────────────────────────────────────────────────────┤
│                    smolnetd                              │
│              (TCP/IP 协议栈，复用 Redox)                  │
├─────────────────────────────────────────────────────────┤
│                 WiFi 驱动 (ath11k)                       │
│              (QCA6390，需要从零实现)                      │
├─────────────────────────────────────────────────────────┤
│                    MHI 总线                               │
│           (Modem Host Interface，高通专用)               │
├─────────────────────────────────────────────────────────┤
│                    PCIe 驱动                              │
│              (总线层，需要实现/移植)                       │
├─────────────────────────────────────────────────────────┤
│               Redox 内核 (aarch64)                       │
│              (微内核，需要适配)                           │
├─────────────────────────────────────────────────────────┤
│                 UART 驱动                                 │
│              (调试输出，首先实现)                         │
├─────────────────────────────────────────────────────────┤
│            Pixel 5 (redfin) 硬件                         │
│         Snapdragon 765G (SM7250) / 8GB RAM              │
│         WiFi: QCA6390 (ath11k)                           │
└─────────────────────────────────────────────────────────┘
```

---

## 目标硬件规格

| 组件 | 规格 |
|------|------|
| 设备代号 | redfin |
| SoC | Qualcomm Snapdragon 765G (SM7250) |
| CPU | Kryo 475 (ARM Cortex-A76 + Cortex-A55), aarch64 |
| GPU | Adreno 620 (不需要) |
| RAM | 8 GB LPDDR4X |
| 存储 | 128 GB UFS 2.1 |
| WiFi | QCA6390 (802.11ax, ath11k 驱动) |
| 蓝牙 | 集成于 QCA6390 (不需要) |
| 调试 | UART (通过 USB-C) |

---

## 技术路线

### 基于 Redox OS

选择 Redox OS 作为基础的原因：
1. **纯 Rust**：与 AgentOS 代码风格一致
2. **微内核架构**：驱动在用户空间，易于调试和开发
3. **Scheme 机制**：Plan 9 风格的资源抽象，设计优雅
4. **已有 aarch64 支持**：树莓派 3B+ 已验证可行

### 关键挑战

| 挑战 | 难度 | 说明 |
|------|------|------|
| UART 驱动 | ★★☆ | 高通串口，调试基础 |
| PCIe 驱动 | ★★★ | 总线层，需要适配 |
| MHI 总线 | ★★★★ | 高通专用协议，无 Rust 参考 |
| WiFi 驱动 | ★★★★★ | ath11k/QCA6390，最复杂的部分 |
| 固件加载 | ★★★ | WiFi 需要加载 vendor firmware |

---

## 开发阶段

### Phase 0: 环境搭建与研究 (1-2 周)

**目标**: 建立开发环境，深入理解 Redox 架构

**任务清单**:
- [ ] 搭建 Redox 编译环境 (macOS/Linux)
- [x] 阅读 Redox 内核源码
  - [ ] `kernel` 仓库：内核核心
  - [x] `base` 仓库：驱动架构 (已归档的 `drivers` 仓库已合并到 base)
  - [ ] `relibc` 仓库：C 库实现
- [x] 研究现有网络驱动实现
  - [x] e1000d (Intel 网卡) - 已分析源码
  - [ ] rtl8168d (Realtek 网卡)
  - [x] virtio-netd (虚拟化网卡) - 已分析源码
  - [x] driver-network crate - 已分析 NetworkAdapter trait
  - [x] netstack (smolnetd) - 已理解网络栈集成方式
- [ ] 研究 aarch64 启动流程
  - [ ] U-Boot 加载
  - [ ] 设备树 (DTB) 处理
  - [ ] 内核入口点
- [ ] 研究 Linux ath11k 驱动源码
  - [ ] 理解 MHI 协议
  - [ ] 理解 QCA6390 固件交互
  - [ ] 整理关键数据结构

**交付物**:
- [x] 开发环境文档 → 见 `AGINX_DRIVER_RESEARCH.md`
- [x] Redox 架构理解笔记 → 见 `AGINX_DRIVER_RESEARCH.md`
- [ ] ath11k 驱动分析文档

**研究笔记**: 详见 [AGINX_DRIVER_RESEARCH.md](./AGINX_DRIVER_RESEARCH.md)

**已确认的关键发现**:
1. 驱动是用户空间 daemon，不是内核模块
2. 使用 `NetworkAdapter` trait 实现网络驱动接口
3. `NetworkScheme` 包装 adapter 并暴露为 scheme
4. smolnetd 自动发现 `network.*` scheme 并集成 TCP/IP 栈
5. QCA6390 需要 MHI 协议，无 Rust 参考实现，需从头编写

---

### Phase 1: QEMU aarch64 验证 (2-3 周)

**目标**: 在 QEMU ARM64 虚拟机上运行 Redox，验证网络功能

**任务清单**:
- [ ] 构建 Redox aarch64 版本
  ```bash
  make ARCH=aarch64 CONFIG_NAME=minimal
  ```
- [ ] 在 QEMU 中启动
  ```bash
  make ARCH=aarch64 qemu
  ```
- [ ] 验证基础功能
  - [ ] 内核启动
  - [ ] 用户空间 shell
  - [ ] 网络连接 (virtio-net)
- [ ] 研究 virtio-net 驱动实现
- [ ] 理解 smolnetd 网络栈

**技术要点**:
```
QEMU aarch64 配置:
- Machine: virt
- CPU: max (模拟 Cortex-A57)
- 网络: virtio-net 或 e1000
- 固件: UEFI (AAVMF) 或 U-Boot
```

**交付物**:
- QEMU 启动成功的镜像
- 网络功能验证报告

---

### Phase 2: Pixel 5 内核适配 (2-4 周)

**目标**: 让 Redox 内核在 Pixel 5 上启动，实现 UART 调试输出

**任务清单**:
- [ ] 研究 Pixel 5 启动流程
  - [ ] Bootloader (ABL) 结构
  - [ ] 内核加载方式
  - [ ] 设备树来源
- [ ] 参考 PostmarketOS 的 redfin 配置
  - [ ] 设备树文件 (dts)
  - [ ] 内核配置 (defconfig)
- [ ] 适配 Redox 内核
  - [ ] 添加 redfin 目标配置
  - [ ] 处理设备树
  - [ ] 配置内存映射
- [ ] 实现 UART 驱动
  - [ ] 高通 MSM UART (8250 兼容)
  - [ ] 早期调试输出
- [ ] 构建可刷机的 boot.img
- [ ] 实际刷机测试

**技术要点**:
```
Pixel 5 启动流程:
1. Boot ROM → ABL (Android Bootloader)
2. ABL 加载 boot.img
3. boot.img 包含: kernel + dtb + ramdisk
4. 内核启动，挂载 rootfs

UART 基地址: 0xa9000000 (需要确认)
```

**交付物**:
- 可刷机的 boot.img
- UART 调试输出正常

---

### Phase 3: 总线层实现 (3-4 周)

**目标**: 实现 PCIe 和 MHI 总线驱动

**任务清单**:
- [ ] 研究高通 PCIe 控制器
  - [ ] 寄存器映射
  - [ ] 初始化序列
  - [ ] 参考 Linux qcom-pcie 驱动
- [ ] 实现 PCIe 驱动
  - [ ] 控制器初始化
  - [ ] 设备枚举
  - [ ] DMA 映射
- [ ] 研究 MHI 协议
  - [ ] 通道结构
  - [ ] 环形缓冲区
  - [ ] 事件/命令/数据通道
- [ ] 实现 MHI 总线驱动
  - [ ] 初始化序列
  - [ ] 通道管理
  - [ ] 电源管理

**技术要点**:
```
MHI (Modem Host Interface):
- 高通设计的通信协议
- 用于 Host 和 Modem/WiFi 之间的通信
- 基于 PCIe 的传输层
- 环形缓冲区 (Ring Buffer) 架构

关键通道:
- MHI_CMD: 命令通道
- MHI_EVT: 事件通道
- MHI_TR: 传输通道 (IP, DIAG, etc.)
```

**交付物**:
- PCIe 驱动 (用户空间 daemon)
- MHI 总线驱动
- 可枚举到 QCA6390 设备

---

### Phase 4: WiFi 驱动实现 (4-8 周)

**目标**: 实现 QCA6390/ath11k WiFi 驱动

这是最复杂的阶段，需要：
1. 理解 802.11 协议基础
2. 实现 mac80211 框架的等价物
3. 处理固件加载
4. 实现数据包收发

**任务清单**:
- [ ] 深入研究 Linux ath11k 驱动
  - [ ] PCI 设备探测
  - [ ] 固件加载机制
  - [ ] WMI (Wireless Management Interface)
  - [ ] HTT (Hardware Transport) 数据路径
- [ ] 设计 Rust 版驱动架构
  ```
  ath11kd/src/
  ├── main.rs        # 入口，PCI daemon 注册，事件循环
  ├── device.rs      # Ath11kDevice，实现 NetworkAdapter trait
  ├── pci.rs         # PCIe 设备操作，BAR 映射
  ├── mhi/
  │   ├── mod.rs     # MHI 总线模块
  │   ├── ring.rs    # 环形缓冲区实现
  │   ├── channel.rs # 通道管理
  │   └── firmware.rs # 固件下载
  ├── wmi/
  │   ├── mod.rs     # WMI 模块
  │   ├── commands.rs # 命令定义
  │   └── events.rs  # 事件处理
  ├── htt/
  │   ├── mod.rs     # HTT 数据路径
  │   ├── tx.rs      # 发送路径
  │   └── rx.rs      # 接收路径
  └── registers.rs   # 寄存器定义
  ```
- [ ] 实现固件加载
  - [ ] 从文件系统加载
  - [ ] 通过 MHI 下载到设备
  - [ ] 验证固件校验和
- [ ] 实现基础 WMI 命令
  - [ ] 设备初始化
  - [ ] 扫描请求
  - [ ] 连接/断开
- [ ] 实现数据包收发
  - [ ] HTT 数据路径
  - [ ] 接收中断处理
  - [ ] 发送队列管理
- [ ] 对接 smolnetd
  - [ ] network scheme 接口
  - [ ] 以太网帧封装

**技术要点**:
```
ath11k 驱动分层:
┌─────────────────────────────────┐
│   NetworkAdapter trait          │  ← Redox 接口
├─────────────────────────────────┤
│   HTT (Hardware Transport)      │  ← 数据包收发
├─────────────────────────────────┤
│   WMI (Wireless Management)     │  ← 扫描/连接控制
├─────────────────────────────────┤
│   MHI (Modem Host Interface)    │  ← 高通专用协议
├─────────────────────────────────┤
│   PCIe                          │  ← 总线层
└─────────────────────────────────┘

QCA6390 PCI 设备信息:
- Vendor ID: 0x17CB (Qualcomm)
- Device ID: 0x1101
- Class: 0x02 (Network controller)
- Subclass: 0x80 (Other)

固件文件 (需要从 Android 提取):
- amss.bin
- m3.bin
- board-2.bin (BDF, Board Data File)
```

**交付物**:
- ath11kd 驱动 daemon
- 可扫描 WiFi 网络
- 可连接 WiFi 网络
- 可收发网络数据包

---

### Phase 5: 网络栈与 Agent 整合 (2-3 周)

**目标**: 完整网络功能，整合 Agent 运行时

**任务清单**:
- [ ] 配置 smolnetd 网络栈
  - [ ] DHCP 客户端
  - [ ] DNS 解析
  - [ ] TCP/UDP socket
- [ ] 移植 SSH 服务器
  - [ ] 评估 openssh vs dropbear vs russh
  - [ ] 编译适配
  - [ ] 配置自动启动
- [ ] 整合 AgentOS runtime
  - [ ] 移植 agentos 核心代码
  - [ ] 适配 Redox 系统调用
  - [ ] 配置自动启动
- [ ] 实现 init 系统
  - [ ] 启动顺序: 内核 → 驱动 → 网络 → ssh → agent
  - [ ] 服务监控和重启

**启动流程**:
```
1. 内核启动
2. init (PID 1)
   ├── pcid-spawner (PCI 设备)
   ├── ath11kd (WiFi 驱动)
   ├── smolnetd (网络栈)
   ├── dhcpd (获取 IP)
   ├── sshd (SSH 服务器)
   └── aginx (Agent runtime)
```

**交付物**:
- 完整可用的系统镜像
- SSH 可连接
- Agent 可运行

---

### Phase 6: 整合测试与优化 (2 周)

**目标**: 系统稳定性测试，问题修复

**任务清单**:
- [ ] 稳定性测试
  - [ ] 长时间运行 (24h+)
  - [ ] WiFi 断线重连
  - [ ] 内存泄漏检测
- [ ] 性能优化
  - [ ] 启动时间优化
  - [ ] 网络吞吐量优化
- [ ] 文档完善
  - [ ] 编译指南
  - [ ] 刷机指南
  - [ ] 开发者文档

**交付物**:
- 稳定的系统镜像
- 完整的文档

---

## 关键技术细节

### Redox 驱动开发模板

```rust
// 驱动 daemon 示例
use redox_scheme::Scheme;
use syscall::error::Result;

struct WifiScheme {
    // 设备状态
}

impl Scheme for WifiScheme {
    fn open(&self, path: &str, flags: usize) -> Result<usize> {
        // 打开网络接口
    }

    fn read(&self, id: usize, buf: &mut [u8]) -> Result<usize> {
        // 接收数据包
    }

    fn write(&self, id: usize, buf: &[u8]) -> Result<usize> {
        // 发送数据包
    }
}

fn main() {
    // 1. 初始化硬件
    // 2. 加载固件
    // 3. 注册 scheme: network:
    // 4. 事件循环
}
```

### PCIe 配置空间访问

```rust
// 通过 /scheme/memory/physical 访问 PCIe 配置空间
// ECAM (Enhanced Configuration Access Mechanism)
// 基地址: 0x10000000 (示例，需要从 DTB 获取)

const PCIE_ECAM_BASE: usize = 0x10000000;

fn read_config(bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    let addr = PCIE_ECAM_BASE
        | ((bus as usize) << 20)
        | ((dev as usize) << 15)
        | ((func as usize) << 12)
        | (offset as usize);
    // 通过 memory scheme 读取
}
```

### MHI 通道结构

```rust
struct MhiChannel {
    id: u32,
    chan_type: ChannelType,
    ering: EventRing,      // 事件环形缓冲区
    tre_ring: TransferRing, // 传输环形缓冲区
}

struct TransferRing {
    base: *mut Tre,  // Transfer Ring Element
    size: usize,
    wp: usize,  // Write Pointer
    rp: usize,  // Read Pointer
}
```

---

## 参考资源

### 已完成的研究文档
- **[AGINX_DRIVER_RESEARCH.md](./AGINX_DRIVER_RESEARCH.md)** - Redox 驱动架构研究笔记
  - NetworkAdapter trait 定义和用法
  - e1000d/virtio-netd 驱动模板
  - PCI 配置文件格式
  - smolnetd 网络栈集成

### 关键 Rust Crates (redox-os/base)
| Crate | 用途 |
|-------|------|
| `driver-network` | NetworkAdapter trait, NetworkScheme 包装器 |
| `pcid_interface` | PCI 设备访问, BAR 映射 |
| `redox_scheme` | Scheme 实现 (Socket, Response) |
| `event` | 事件队列 (IRQ/scheme 事件) |
| `common` | 日志, DMA 分配工具 |
| `libredox` | 低级 Redox 系统调用 |
| `syscall` | 系统调用定义 |

### Redox OS
- [Redox Book](https://doc.redox-os.org/book/)
- [Kernel Source](https://gitlab.redox-os.org/redox-os/kernel)
- [Drivers Source](https://gitlab.redox-os.org/redox-os/drivers)
- [Schemes Documentation](https://doc.redox-os.org/book/schemes.html)

### Pixel 5 / Snapdragon
- [PostmarketOS redfin Wiki](https://wiki.postmarketos.org/wiki/Google_Pixel_5_(google-redfin))
- [Linux qcom-pcie driver](https://github.com/torvalds/linux/blob/master/drivers/pci/controller/dwc/pcie-qcom.c)
- [Linux ath11k driver](https://github.com/torvalds/linux/tree/master/drivers/net/wireless/ath/ath11k)

### WiFi / MHI
- [ath11k MHI implementation](https://github.com/torvalds/linux/tree/master/drivers/net/wireless/ath/ath11k)
- [MHI Bus driver](https://github.com/torvalds/linux/tree/master/drivers/bus/mhi)
- [QCA6390 firmware](https://github.com/kvalo/ath11k-firmware)

### 802.11 协议
- [IEEE 802.11 Wikipedia](https://en.wikipedia.org/wiki/IEEE_802.11)
- [Linux mac80211 framework](https://www.kernel.org/doc/html/latest/driver-api/80211/mac80211.html)

---

## 风险评估

| 风险 | 可能性 | 影响 | 缓解措施 |
|------|--------|------|----------|
| 固件不可用 | 低 | 高 | 从 Android 系统提取 |
| MHI 协议复杂 | 高 | 高 | 深入研究 Linux 驱动 |
| 硬件文档缺失 | 中 | 中 | 参考开源驱动 + 逆向 |
| Redox 内核 bug | 中 | 中 | 准备向 Redox 社区提 PR |
| 时间超出预期 | 高 | 中 | 分阶段交付，MVP 优先 |

---

## 里程碑总结

| 阶段 | 时间 | 关键成果 |
|------|------|----------|
| Phase 0 | 1-2 周 | 环境搭建，架构理解 |
| Phase 1 | 2-3 周 | QEMU 验证通过 |
| Phase 2 | 2-4 周 | Pixel 5 启动 + UART |
| Phase 3 | 3-4 周 | PCIe + MHI 就绪 |
| Phase 4 | 4-8 周 | WiFi 可用 |
| Phase 5 | 2-3 周 | SSH + Agent 整合 |
| Phase 6 | 2 周 | 稳定版本 |
| **总计** | **16-26 周** | **完整可用的 aginx** |

---

## 下一步行动

1. **立即开始**: Phase 0 - 环境搭建
2. **本周目标**:
   - [ ] 搭建 Redox 编译环境
   - [ ] 阅读 e1000d 驱动源码
   - [ ] 研究 ath11k 驱动结构
3. **决策点**:
   - 是否需要先在 QEMU 上验证？
   - 固件从哪里获取？

---

*文档版本: v1.0*
*创建日期: 2026-03-24*
*项目: aginx - Agent OS for Pixel 5*
