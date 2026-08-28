# Pixel 5 裸机 4G 联网开发计划

## 目标

在 Pixel 5 上通过 SIM 卡 + 4G 基带实现网络连接，支持 SSH/telnet 远程登录 shell。

## 架构概览

```
用户 (电脑) ──TCP/IP──> Pixel 5 smoltcp
                            ↑
                      rmnet/PPP 数据包
                            ↑
                    X52 基带 (独立处理器, 自带固件)
                            ↑
                    PMIC 上电 + UART AT 指令拨号
```

**关键原理**：基带是独立处理器，自带固件处理全部 4G 射频和协议栈。
我们只需要：上电 → AT 拨号 → 收发 IP 包。不写任何射频/协议层代码。

## 前置条件（已有基础）

- ✅ Pixel 5 内核启动到 shell
- ✅ Framebuffer 显示 + 滚动 console
- ✅ smoltcp TCP/IP 栈（QEMU 上已验证 DHCP/TCP/ICMP）
- ✅ QUP UART 驱动（存在但 init 卡住，需修复）
- ✅ MMU 页表代码（已写好但未启用）
- ✅ ath11k 骨架（MHI 寄存器定义、ring 结构可复用）

## 分阶段实施

### 第一阶段：基础设施修复（前置依赖）

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 1.1 | 修复 QUP UART | 调试 init 卡住问题（可能是 QUPv3 寄存器偏移或时钟未启用），使 UART 可用 | 中 |
| 1.2 | 启用 MMU | 调用已写好的 `init_redfin()`，启用 cache 加速 framebuffer 和后续 DMA | 小 |
| 1.3 | Frame allocator | 适配 redfin：RAM_START=0x80000000，正确计算可用页数 | 小 |

**验证标准**：MMU 启用后内核正常运行，UART 能输出字符。

### 第二阶段：基带硬件初始化

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 2.1 | SPMI 总线驱动 | 实现 SPMI controller（0x0C00_0000 区域）的基本读写，能访问 PMIC 寄存器 | 大 |
| 2.2 | PMIC 基带上电 | 通过 SPMI 写 PM7250B 寄存器，给基带供电、解除复位 | 中 |
| 2.3 | 确认基带 UART | 确定哪个 UART 连接基带（可能是独立于 QUP 的串口），验证 AT 响应 | 中 |

**验证标准**：基带上电后，UART 发 `AT\r\n` 能收到 `OK`。

**技术细节**：
- SPMI arbiter 地址需从 DTB 或硬件文档确认（Linux 用 0x0C440000）
- PM7250B 基带电源控制：可能涉及 LDO/NCP 控制器，需查 PMIC 寄存器手册
- 基带 UART：研究文档提到 0x00A9C000，需验证

### 第三阶段：AT 指令拨号

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 3.1 | AT 命令收发器 | 实现 UART 上的 AT 命令发送和响应解析（行缓冲、`OK`/`ERROR` 检测） | 中 |
| 3.2 | SIM 卡检测 | AT+CPIN? 确认 SIM 卡就绪 | 小 |
| 3.3 | 网络注册 | AT+COPS=0 自动选网，AT+CREG? 查询注册状态 | 小 |
| 3.4 | PDP 上下文 | AT+CGDCONT=1,"IP","cmnet"（移动）/ "3gnet"（联通）/ "ctnet"（电信） | 小 |
| 3.5 | 拨号 | ATD*99# 或 AT+CGDATA="PPP",1 建立数据连接 | 中 |

**验证标准**：AT 拨号成功，基带进入数据模式（不再响应 AT 命令，开始发 PPP 帧）。

**AT 指令序列**：
```
AT                              → OK              (测试通信)
AT+CPIN?                        → +CPIN: READY    (SIM卡就绪)
AT+COPS=0,,7                    → OK              (自动选网, 4G only)
AT+CEREG?                       → +CEREG: 0,1     (已注册4G)
AT+CGDCONT=1,"IP","cmnet"       → OK              (设置APN)
ATD*99#                         → CONNECT         (拨号成功)
```

### 第四阶段：数据路径 + smoltcp

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 4.1 | PPP 帧解析 | 解析基带返回的 PPP 帧（HDLC 封装：0x7E 帧界、转义、FCS 校验） | 大 |
| 4.2 | smoltcp 设备封装 | 实现 `smoltcp::phy::Device` trait，将 PPP 解出的 IP 包喂给 smoltcp | 中 |
| 4.3 | IP 地址获取 | PPP IPCP 协商获取运营商分配的 IP，或解析 DHCP | 中 |
| 4.4 | TCP 连接验证 | 用 smoltcp 发起 TCP 连接测试网络连通性 | 中 |

**验证标准**：能获取 IP 地址，能 ping 通外部服务器。

**PPP 帧格式**：
```
0x7E [addr 0xFF] [ctrl 0x03] [protocol 2B] [data...] [FCS 2B] 0x7E
```
- 协议 0x0021 = IP 数据
- 协议 0xC021 = LCP (链路控制)
- 协议 0x8021 = IPCP (IP 控制协议，用于获取 IP)

### 第五阶段：远程 Shell

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 5.1 | TCP 监听 | 在指定端口（如 2323）监听 TCP 连接 | 小 |
| 5.2 | Telnet 协议 | 实现 minimal telnet（处理 IAC 协商、字符回显） | 中 |
| 5.3 | Shell 集成 | TCP 连接上运行的 shell（复用 QEMU 版本的命令处理） | 小 |
| 5.4 | Framebuffer 状态 | 屏幕显示网络状态（IP、连接数、信号强度） | 小 |

**验证标准**：电脑通过 `telnet <手机IP> 2323` 连入，执行 help、version 等命令。

### 第六阶段：音量键控制（辅助功能）

| # | 任务 | 描述 | 预估 |
|---|------|------|------|
| 6.1 | PMIC 按键检测 | 通过 SPMI 读取 PMIC PON 寄存器检测音量/电源键 | 中 |
| 6.2 | 按键映射 | 电源键=重启，音量上+电源=关机，音量下=显示网络状态 | 小 |

## 风险与未知

| 风险 | 影响 | 缓解 |
|------|------|------|
| 基带需要加载固件（PIL） | 需要额外实现固件加载 | 先验证 ABL 是否已加载；或从 eMMC 读取 |
| SPMI 地址不确定 | 调试周期长 | 从 DTB 提取准确地址；参考 Linux DTS |
| 基带 UART 地址不确定 | 找不到通信通道 | 用 `fastboot oem dmesg` 查 ABL 日志 |
| PPP 协商复杂 | 数据路径延迟 | 先用最简 IPCP，不加密不压缩 |
| UART 带宽低（115200bps） | 网速慢 | 先验证功能，后续升级 PCIe/MHI 数据路径 |
| 基带未上电导致无响应 | AT 命令超时 | 先确保 PMIC 上电正确，用示波器或串口日志确认 |

## 开发优先级

```
阶段1 (基础设施) → 阶段2 (基带硬件) → 阶段3 (AT拨号)
                                           ↓
                                     阶段4 (数据路径) → 阶段5 (远程Shell) → 阶段6 (按键)
```

**最小可行路径**：阶段 1→2→3→4→5，约 3-5 个工作日。
**完整路径**：加上阶段 6 和 PCIe 高速数据，约 1-2 周。

## 立即行动

1. [ ] 修复 QUP UART init（调查卡住原因）
2. [ ] 启用 MMU（取消 init_redfin 的 no-op）
3. [ ] 从 DTB 提取 SPMI 和 PMIC 地址
4. [ ] 实现 SPMI 基本读写，验证 PMIC 可访问

---
*创建日期：2026-04-02*
*基于 4G.txt 研究文档和现有代码分析*
