# AginxOS 系统设计（SYSTEM）

Status: **draft v1**（2026-08-29，与用户逐条对齐后落笔）。本文是系统形态的权威描述；
与 DECISIONS.md 冲突处以本文更新日期更晚的条款为准，并在下文显式标注 supersede。

## 0. 与 DECISIONS.md 的关系

- **Supersede DECISIONS §1 "No package manager, no app installer"（2026-08-29）**：
  当时的前提是"封闭 appliance，能力扩展只走 agent socket"。产品目标明确为
  **智能体手机 + Rust CLI 平台**后，安装能力成为系统的一等功能，由本文 §5 的
  统一安装器承担。其余约束不变：只允许静态 musl aarch64 二进制，无动态加载器、
  无 APK、无通用包管理器——"别的软件装不进来"仍然是平台的自然属性而非刻意限制。
- **细化 DECISIONS §4（Product role）**：§4 说"agent 是系统接口而非普通 app"，
  现细化为 §2 的两档软件模型——aginx 栈是**必须存在档软件**，不烙进系统底座。
- **沿用 DECISIONS §5（UI stack）**：aterm 就是 §5 "Early: DRM/KMS 直绘 + evdev"
  的具体化；"可选 Wayland"条款继续后置。

## 1. 定位

AginxOS = 跑在手机上的 agent 节点操作系统。终局产品：**智能体手机**——
分身跟着设备走（开机=在线，关机=离线），手机是自己在 aginx 互联网上的家。

- 核心目标：**AginxOS + Wi-Fi**。蜂窝移动网络是加分项，不挡主路
  （电信入库问题已文档化，见 HARDWARE.md M6/M7）。
- 在 aginx 生态中的形态：能力上等同 ARCHITECTURE.md §11.3 矩阵的**桌面形态**
  （真 Linux、有本地 CLI、可 24h 在线），不走 uniffi 移动嵌入路径。
- 生态铁律全部沿用：relay 保持笨、网关保持 nginx、改协议同批三端+ACP.md。

## 2. 分层模型

**内核/rootfs 底座之上，一切软件化。系统不自带任何软件；**
**首次初始化时连网下载最新版。** 软件分两档：

```
┌─ 底座（rootfs 构建脚本烙入，userdata ext4 持久）────────────┐
│  内核 + 忙盒 busybox + aterm 终端 + 安装器 + updater         │
│  （这套东西负责开机、联网、装软件、升级自己——仅此而已）        │
└──────────────────┬─────────────────────────────────────────┘
                   │ 安装器下载（manifest + sha256，GitHub releases）
        ┌──────────┴───────────┐
        ▼                      ▼
  必须存在档                自由安装档
  · aginx 网关（如 nginx）   · codex（官方 musl 二进制）
  · aginxbrowser（如 IE）    · grok-build（源码自建 musl）
  · aginx-carrier（分身runtime）· …仅限 Rust CLI
```

- **必须存在档**：aginx（前门/路由）、aginxbrowser（本机上网能力，大杀器）、
  aginx-carrier（分身 runtime）。自家全套出厂必在——"必在"由首启初始化下载保证，
  不由镜像捆绑保证。carrier 虽自家开发，在系统里按软件对待：可装、可升、可换版本。
- **自由安装档**：codex、grok-build 等第三方 Rust CLI。codex/grok 本机使用 =
  独立应用直接在 aterm 里跑；**只有外界连入时才走 aginx 网关的 ACP 那套**
  （注册 PATH agent + 方言翻译，机制同生态桌面网关接 claude）。
- **为什么必须存在档也不进镜像**：boot 分区 96MB。内核 + 底座之外，
  carrier 单二进制 ~29MB、aginxbrowser 内嵌 V8 预计 80MB+——算术上装不下。
  首启下载同时还天然保证"出厂即最新"。

## 3. 组件职责

| 组件 | 职责 | 边界 |
|---|---|---|
| **aterm** | 系统主 UI：pty + vte 解析（alacritty 的 parser crate）→ 字符网格 → fontdue → DRM 扫描输出；屏幕键盘（触摸→按键事件）；启动器（分身对话/codex/grok/sh）；触摸滚动回翻 | 黑底绿白字（磷光终端风，用户钦定配色）；不是通用 GUI 栈，永远不做窗口管理 |
| **aginx 网关** | agent:// 寻址路由、relay 出站长连（relay.aginx.net，穿 NAT）、ACP stdio 桥、friends/JWT 鉴权、会话台账 | 保持 nginx：业务逻辑一律在 carrier |
| **aginxbrowser** | 本机 loopback HTTP + MCP：fetch/search/会话交互，agent 的眼睛。分身和 codex（MCP native）直接接 | 手机版裁 feature：v1 无 stealth（BoringSSL 交叉编译）、无 screenshot（fontconfig 动态依赖）；fetch/search/session/MCP 全在默认栈 |
| **aginx-carrier** | 分身 runtime：定义/记忆（本地 SQLite）/编排，LLM 走 brain；本机分身对话经 ACP | 数据全落 /var；升级不刷机 |
| **安装器** | 统一包管线：manifest（URL+sha256+服务声明+网关注册钩子）→ 下载 → 校验 → tmp+rename 原子替换 → 守护类注册/重启 | 唯一安装入口；保留上一份二进制供回滚 |
| **updater** | 系统升级（§6）。本身也是一个 app | 二期，初版系统升级仍走 fastboot |

## 4. 目录与持久化

rootfs 在 userdata ext4 上，全盘持久（重启不丢；userdata 清空=出厂重置）。

```
/etc/            系统配置：wifi.conf、brain 端点+key（0600）、安装源 manifest 指针
/usr/bin/        底座自带（busybox、aterm、安装器、updater）
/var/bin/        全部软件二进制（必须存在档 + 自由档同目录，manifest 记档位）
/var/apps/<app>/ 各 app 数据目录
/var/home/       HOME——所有 CLI 的 ~/.codex、~/.aginx 等自然落这里
/var/workspace/  系统工作目录：codex/grok 的 cwd、agent 产出、用户文件
/var/log/        系统与 app 日志
```

## 5. 网络与算力

- **Wi-Fi 主链路**（M5 已通：自动连接 + 联网验证）。
- **蜂窝**：加分项。设备上 m7 racer 持续重试 START_NET，随机放行窗口自动配好
  rmnet + DNS + `cell ok` 写 boot.state；不投入更多精力。
- **relay.aginx.net**：网关出站长连，手机分身经 `agent://<id>.relay.aginx.net/<分身>`
  可被借访；入站全走 relay，NAT 无所谓。
- **brain**：OpenAI 格式 API 端点（开源可自部署），配置项 = base_url + key，
  写 /etc（0600）。carrier、需要 LLM 的 app 共用。
- **校时**：TLS 对时钟敏感——Wi-Fi 起来后先校时（busybox ntpd），
  再允许 relay/TLS 握手。加入 init 顺序依赖。
- **CA 根证书**进 rootfs（rustls 系 app 用 webpki-roots 则不需要，逐个确认）。

## 6. 更新机制

### 6.1 软件更新（统一管线，一期即做）

包 = 静态 musl aarch64 二进制 + manifest：

```toml
[[app]]
name = "aginx-carrier"
tier = "required"            # required | optional
source = "github:yinnho/aginx-carrier"
asset = "aginx-carrier-aarch64-unknown-linux-musl.tar.gz"
service = { autostart = true }   # 守护类声明
```

- 安装/升级同一路径：下载 → sha256 → tmp+rename → 守护类重启。
- 回滚：保留上一份二进制，切回。
- 必须存在档的"必在"语义 = **安装器保证存在**：首启下载；此后每次 boot 校验，
  缺失/损坏自动重下（自愈）。/var 清空 = 出厂重置 = 重新初始化。
- 更新源：自家件 GitHub releases；codex 用官方 musl 资产；grok-build 无官方
  release，自建 musl 后挂到我们自己的 release。
- 签名后置；一期 TLS + sha256。

### 6.2 系统升级（内核+底座）

- **开发期（现状）**：fastboot 刷 boot.img；`.factory/` flash-all 兜底。
- **产品形态（二期）**：A/B 双槽设备自更新——updater app 拉 manifest →
  下载 boot.img → 写**非活动槽**（先读当前槽，只写对面）→ 切槽重启 →
  启动成功标记 successful；失败则 retry counter 耗尽自动回落旧槽。
  vendor_boot 仅模块/dtb 变化时随同。/var 不动，数据与软件全存活。
- **统一视角**：系统升级也是一个"包"，updater 是普通 app，
  只是安装动作=写槽位而非落 /var/bin。

## 7. 安全与凭证

- 三层凭证沿用生态模型：relay secret / 网关 token / borrower（ACP.md §2）。
  全部落 /etc 或 /var/home/.aginx，0600，永不入库。
- 安装器是一等攻击面：manifest 源固定、sha256 必验；签名机制二期。
- codex 的沙箱（bwrap/user namespace）依赖内核支持，**待验证**；
  不支持则以显式配置关闭沙箱跑，记入 HARDWARE.md。
- 手机被借访的配额/名单门由 carrier `[borrow]` 段承担（生态已有三闸语义）。

## 8. 电源

正常手机用法：插电/充电随用户。Rust 守护 + 无 Android 后台 = 待机卖点。
二期：空闲息屏、触摸/消息唤醒、长期插电的充电保护策略。

## 9. 出厂状态与首启流程

1. 开机 → boot card（已有）。
2. **Wi-Fi 设置向导**：扫描附近 AP → 列表选择 → 屏幕键盘输密码 →
   写 /etc/wifi.conf → 连接验证。已有 /etc/wifi.conf 则跳过直连
   （开发期 adb 手放配置的 headless 路径保留）。
3. 校时 → 安装器按 manifest 下载必须存在三件套（aginx/aginxbrowser/carrier）。
4. 注册 + 拉起常驻服务；网关连 relay，手机入网。
5. aterm 启动器就绪：分身对话 / codex（若装）/ sh。
6. brain key 未配置 → 启动器提示配置（首个设置项）。

全程黑底绿白；下载进度在 boot card/向导上可见。
Wi-Fi 向导依赖屏幕键盘 → 与 aterm（M11）同一条 UI 线；
无头开发路径（adb 预置 wifi.conf + manifest）不依赖它。

## 10. 后置项（明确不做进一期）

签名机制、中文输入法（v1 英文/ASCII）、语音（音频子系统未 bring-up）、
updater A/B 自更新、息屏/唤醒电源策略、aginxmemory、stealth/screenshot
feature、蜂窝主线。

## 11. 里程碑拆分

骨干原则（沿用 DECISIONS §1 排序精神）：**先无头后有屏**——
手机先作为网络节点活起来（桌面 agc 能访问手机分身），UI 并行推进。

| # | 里程碑 | 内容 | 验证 |
|---|---|---|---|
| M8 | 手机入网（无头） | aginx/carrier/aginxbrowser musl 构建 + 安装器 v0（adb 手动喂 manifest）+ init 常驻 + relay 长连 | 桌面 `agc agent://<手机id>.relay.aginx.net/<分身>` 打通；HARDWARE.md 记录 |
| M9 | aginxbrowser musl 移植 | deno_core/v8 升级到带官方 musl 预编译的版本（137.3.0 无 musl 资产，新版有）；裁 stealth/screenshot | 手机上 fetch/search/MCP 实测 |
| M10 | 首启初始化 | Wi-Fi 设置向导（扫描/选择/输密码/写 wifi.conf，UI 面与 M11 同线）+ 安装器按 manifest 自动下载必须存在档 + 自愈校验 + 校时入 init | userdata 清空重启 → 向导配网 → 自动恢复到完整节点 |
| M11 | aterm 终端 | pty + vte + 字符网格 DRM 渲染 + 屏幕键盘（ASCII）+ 启动器 | 机上 sh/codex 交互可用；黑底绿白 |
| M12 | codex 安装 | 官方 musl 二进制经安装器落地，沙箱能力实测 | 机上跑通 codex 会话 |
| M13 | grok-build | 源码自建 musl + 安装 | 机上跑通 |
| M14 | updater | A/B 设备自更新 | 刷对面槽重启回滚实测 |

依赖：M9 不挡 M8（browser 后进服务序列）；M10 的向导 UI 依赖 M11 键盘（headless 路径不依赖）；M11 与 M8/M9 全并行。
