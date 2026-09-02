# 分身本体硬件定义（v1 立项，2026-08-31 讨论定型）

## 一句话定位

**卡 = 分身的实体身体**。不是人用的手机（人的手机还是人的手机），
是一台自包含的 agent 计算机：大脑、双手、五官、记忆全在卡上；
服务器收缩为纯消息管道；云只剩模型农场。立在桌上，它就是那个
「他/她」——微信里的账号有了肉身。

## 架构总图

```
┌─ agent 卡（分身本体，全 root，自己签固件）──────────┐
│ aginx (brain: 记忆/调度/路由)                          │
│ AginxBrain (LLM 网关: key/路由/降级) ─────────────────┼──→ 云端各家大模型 API
│ aginx-carrier (hands: 执行/监控/修复)                   │      （纯模型农场，无状态）
│ 小模型: whisper(ASR) + Qwen 小档(离线兜底/唤醒词)       │
│ 感官: 摄像头(眼) · mic×2(耳) · speaker+听筒(嘴)        │
│ 脸: 2.8" AMOLED(状态/表情/出码) · 全套 agpkg 工具      │
│ musl rootfs · 256GB UFS · 开机即绑定(agent:// 出码)    │
└───────────────────────────────────────────────────────┘
        ▲▼ 唯一外部依赖：一条长连接
┌─ 服务器（纯管道，零智能零记忆零计算）───────────────────┐
│ 微信网关（公网回调，必须存在的唯一理由）· relay · DupHub │
└───────────────────────────────────────────────────────┘
```

三条推论：

1. **服务器不再承担计算**——微信回调需要公网域名是它存在的唯一硬理由。
2. **断云只降级**（小模型兜底简单对话/ASR），**断服务器只断微信耳嘴**，
   本机 agent 环境完整活着。唯一上移的恰好是最耗电的部分（LLM 推理）。
3. **记忆在卡里 = 单点失忆风险** → rclone 定期推服务器备份是产品
   必选项，不是可选项（session-events 每日 rsync 先例已有）。

## 产品形态：有点人样

硬件只花几块钱，人格 90% 靠灯和屏渲染（Anki Vector/Cozmo 路线）：

```
┌───────────────────────┐
│        ◉ ˙            │ ← 眼：微凸摄像头(6.5mm) + 环形状态灯
│   ┌───────────────┐   │      （呼吸=活着  亮=聆听  闪=思考）
│   │               │   │
│   │    小屏(脸)     │   │ ← 2.8" AMOLED：眼睛动画/波形/状态
│   │               │   │
│   └───────────────┘   │
│    ¨¨¨¨¨¨¨¨¨¨¨¨¨     │ ← 嘴：长条点阵出音孔（speaker）
│  ·                  · │ ← 耳：mic×2 开孔，底部两角（AEC 用）
└───────────────────────┘
    85.6 × 54 × 6mm（标准卡尺寸，进钱包）
```

- 「人样」的真正杠杆在软件：灯的呼吸节奏、眼睛动画、接话时机。
  硬件只需把解剖学摆对——眼在上（有灯）、屏居中（会动）、嘴在下
  （有孔）、耳在角（有孔）。
- 屏渲染 Rust 直绘 framebuffer 即可，不引 GUI 框架：待机闭眼呼吸、
  聆听睁眼+声波、说话嘴部波形、干活进度条。
- 交互入口：**v1 按需唤醒（触摸/拿起），v2 唤醒词常听**（NPU KWS
  待机 +0.2-0.3W，已拍板分两步）。
- 绑定即激活：卡背面印 `agent://` 设备码，首次开机屏上出码 → 人
  手机扫码 → 绑定到分身。卖卡 = 卖分身的肉身，直接咬合商业闭环
  （分身进化 → 分享活实例 → 远程使用收费）。

## BOM（1 万台档，深圳 ODM 量级）

| 类别 | 明细 | 估价 |
|---|---|---|
| SoC | RK3576（4×A72+4×A53，6 TOPS NPU，4K VPU）+ PMIC | ¥65-75 |
| RAM | 8GB LPDDR4x | ¥80-100 |
| 存储 | 256GB UFS 2.0 | ¥100-130 |
| 屏 | 2.8" AMOLED + 触摸 | ¥40-55 |
| 眼 | 8-13MP 定焦模组 + 状态灯 | ¥12-18 |
| 耳嘴 | mic×2 + micro speaker + 听筒 | ¥10-14 |
| 电池 | 3000mAh 薄软包 + 充电保护（插电常驻 + 80% 限充） | ¥12-16 |
| WiFi/BT | WiFi6 模组（RK3576 需外挂） | ¥14-20 |
| 板级 | 6-8 层 PCB + 被动件 + USB-C + FPC | ¥30-45 |
| 结构 | 壳体/中框/玻璃盖板/石墨铜箔/点阵孔 | ¥28-40 |
| 制造 | SMT + 组装 + 老化 + 治具摊销 + 包装 | ¥45-60 |
| **合计** | | **¥450-570（中位 ~¥500）** |

核心三件（SoC+RAM+存储）占 55%+，**存储是最大变数**（当前历史级
涨价周期，±30% 波动，立项时锁价预购对冲）。降本旋钮（可压到
¥350-400）及其代价：128GB eMMC 省¥60-80（视频暂存紧张）、4GB RAM
省¥40-50（whisper+Qwen+duckdb 不能同开）、LCD 省¥15-25（「人样」
打折）。**结论：首发按 ¥500 全配走**——极简档伤的恰好是差异化。

SoC 选型依据：NPU 现在有三个常驻理由（ASR、离线兜底对话、v2
唤醒词）；VPU 给 ffmpeg 硬编硬解；往上 RK3588S 性能富余但 10W 峰值
6mm 卡压不住，往下 RK3568 无 NPU 「装小模型」即落空。

## 功耗哲学：agent 不赶时间

薄卡被动散热按**平均功率**设计，而 agent 是最不怕慢的用户：

| 层 | 功耗 | 场景 |
|---|---|---|
| 待机（99% 时间） | 0.3-0.8W | 裸 Linux + Rust 全家 = WiFi keepalive，无 Android 框架税 |
| 日常活动 | 1-2W | 消息处理、brain 调度、CLI 小任务、duckdb |
| 峰值（短时，节流兜底） | 4-6W | ffmpeg 转码、whisper、cron 夜轮 |

热预算调度进 carrier：读 thermal zone，热则降并发限频；cron 夜轮
排凌晨限速慢跑（转 30 分钟还是 5 分钟，没人在乎）。

**功耗功劳归属（澄清，防误读）**：省电的是「裸 Linux 无 Android
框架税 + 精简常驻集」这个形态，**不是 Rust**——语言编译产物与 C
同档效率，功耗由内核与硬件决定（cpuidle 深浅、cpufreq 策略、
suspend、WiFi 芯片省电模式、DRAM 自刷新、时钟门控）。Rust 的贡献
是间接的：无 GC/无 VM → 常驻内存几十 MB（DRAM 自刷新是待机大头）
+ 唤醒源少（无 GC/VM 定时唤醒，CPU 躺得住深 idle），让小而稳的
常驻集工程成本低。**是使能者，不是功耗来源**——同一颗芯片跑
Android 与跑裸 Linux 待机差一个量级，语言没变，OS 形态变了。

## 语言分层立法：C 驱动与 Rust 的边界

「驱动」拆两层，**Rust 写到 ioctl 为止**：

```
┌─ 应用（aginx/carrier/表情渲染）────────── 全 Rust
├─ 用户态外设访问层（eye/ear/mouth/face）── 全 Rust ← 我们写的
├─ vendor 库（librockchip_mpp/rga/RKNN）─── C，FFI 包薄绑定
├─ ioctl /dev 边界（v4l2/pcm/drm/mpp）
├─ 内核驱动（ISP/VPU/codec/DSI/USB…）───── C，BSP 现成继承
└─ u-boot / 设备树 ──────────────────────── C + DTS 数据，继承
```

- **内核层留 C（能改不该改）**：RK3576 钉在 vendor BSP 内核（ISP/
  VPU/NPU 主线支持不全），该分支无 Rust for Linux 基建，自维护
  内核 fork 与团队规模不匹配；且 BSP 已把硬件全暴露成 /dev 节点，
  内核驱动是继承不是开发
- **用户态访问层全 Rust（本来就该）**：眼=v4l2r、耳嘴=cpal + speex
  AEC、脸=drm-rs（DRM 本质是 ioctl，rustix 裸写亦可）、GPIO/LED/
  温控=文件读写原生
- **三件 vendor 库 FFI 包薄，unsafe 圈死在 ffi/**：librockchip_mpp/
  librga/RKNN runtime 是 vendor 黑盒（文档不全、magic number 多），
  重写纯亏；薄绑定 + RAII guard 包 fd/buffer 生命周期（MPP buffer
  与 DMA-BUF 跨库传递是风险集中点）。ffmpeg 走 MPP 版当 CLI 子进程
  调，不包 FFI

新 crate `periph`（五官各一模块，对外全安全 API；Termux 阶段 -1
五官模块 stub 掉，系统层照跑——不依赖具体硬件形态；crate 形状与
平台分文件参照 [OPENLOGI-STUDY.md](OPENLOGI-STUDY.md)——18k★
生产级同类实证）：

```
crates/periph/
  eye.rs      V4L2 拍照 → NV12/JPEG
  ear.rs      cpal 采集 + speex AEC 前端 → f32 帧
  mouth.rs    TTS 音频播放队列
  face.rs     DRM 直绘（表情状态机消费）
  npu.rs      RKNN FFI（whisper/Qwen 加载与推理调度）
  thermal.rs  温度读取 + 热预算接口（carrier 调度消费）
  ffi/        vendor 绑定，unsafe 只出现在这
```

## 成本三本账（2026-08-31 估算，立项 RFQ 三家报价校准）

**NRE（ODM 全定制一次性）**：

| 项 | 估价 |
|---|---|
| ID/MD 设计（前脸/凸台/6mm 堆叠是难点） | ¥30-80 万 |
| 电子设计 + Layout + 2-3 轮打样 | ¥30-60 万 |
| 驱动 bring-up + AEC 音频联调（合同单列） | ¥20-50 万 |
| 模具 | ¥20-40 万 |
| 认证（SRRC + CCC，无蜂窝） | ¥8-15 万 |
| **合计** | **¥120-250 万** |

**分阶段投入（每步可停）**：

| 阶段 | 内容 | 投入 | 周期 |
|---|---|---|---|
| **-1 旧手机** | Pixel 5 Termux/proot 系统层全量验证（见下节） | **~¥0（手头机）** | 即刻 |
| 0 验证 | RK3576 开发板 ×5 + USB 五官套件，系统层全跑通 | **<¥1 万** | 1-2 个月 |
| 1 白牌 | 500 台半定制（改壳+预装），小批量探需求 | ¥60-90 万 | +2-3 个月 |
| 2 ODM 量产 | 1 万台全定制 | **¥700-850 万** | 6-9 个月 |

阶段 0 强调：软件资产（rootfs/表情渲染/开机绑定/agpkg/ffmpeg MPP）
100% 在开发板上做完，NRE 一分不花。**真正贯穿全程的成本是我们自己
的工程时间**，钱的大头在阶段 2 才发生。

## 阶段 -1：旧手机先行验证（进行中，2026-08-31 起）

比开发板还快的一步——旧手机直接开跑，系统层 90% 当天可验：

- **当前测试机：Pixel 5（redfin，骁龙 765G）**——性能档几乎正对
  RK3576（A76×2+A55×6 vs A72×4+A53×4，单核还更强），跑出来的
  结论可直接外推真机。
- 分两层推进：
  1. **Termux + proot 挂 musl rootfs**（零刷机）：aginx/AginxBrain/
     carrier + agpkg 全量跑——系统层功能验证主战场。注意 musl
     动态件（opencode/duckdb）依赖 rootfs 自带的
     `/lib/ld-musl-aarch64.so.1`，所以必须整 rootfs 进 proot，
     不能裸跑静态件就完事
  2. **解锁 + root**（Pixel 解锁最松，`fastboot flashing unlock`
     一条命令）：完整 rootfs 常驻、开机自启、功耗曲线；再往上可
     刷 pmOS（sm7250 mainline 支持好）体验裸 Linux 形态
- **proot/chroot 分层铁律**（seg6 CMF Phone 1 手机服务器实战课，
  2026-09）：proot 的用户态 syscall 翻译开销对进程启动频繁/文件
  访问密集的负载会成为瓶颈（其场景 Chrome 失真明显），同一套
  文件系统 root 后换真 chroot 性能即恢复。落到我们：
  **proot 层只测功能正确性**（流程跑通与否），**一切性能/功耗
  数字（ffmpeg 转码、whisper 推理耗时）必须在 chroot 层测**才可
  外推 RK3576——文件系统同一个，只换挂载方式
- **pmOS 降级为条件项**：同篇实测 CMF Phone 1 刷 pmOS 后 Wi-Fi/
  蓝牙/硬加速全废且黑砖一次——mainline 支持质量逐机型差异巨大，
  Pixel 5（sm7250）须先确认社区成熟度再动，非默认步骤
- **Tailscale 做 SSH 运维通道**：阶段 -1 手机在家 NAT 后面，
  ssh 进不去就没法远程调试；真机上等价物是 relay 长连接，但
  阶段 -1 调试期 Tailscale 最省事
- **五官不在旧手机上测**：摄像头/音频驱动是 RK3576 平台的事，
  旧手机验证不了；要测走 USB 外设（OTG）顶，正式验证在阶段 0
- 两个坑：插电常驻配 ACC 限充 80% 防鼓包；Termux 被杀后台用
  wake-lock + 通知前台服务顶（Pixel 原生系本来就好）
- 备用机型（如需多台并行）：一加 6/6T（pmOS 最成熟社区机）、
  Poco F1（便宜，但小米解锁要等待期）；出局：华为 2018 后全系
  （bootloader 不可解锁）、32 位老机（全线 aarch64 架构不符）

收入参照：¥500 BOM → 零售 999-1299（DTC）毛利 50% 档；卡是生态
入口，硬件不追暴利，分身服务订阅是正餐。

## 安全与身份（v1 必做，不可事后补）

- **Secure boot**：出厂 OTP 烧死公钥，BootROM→bootloader→kernel→
  rootfs 逐级验签；LLM API keys + 分身记忆加密存储，密钥派生绑定
  芯片。效果：卡丢=别人拿到砖，key 与记忆不泄露。
- **开机即绑定**（与 secure boot 一静一动互补）：首开出 `agent://`
  码 → 扫码 → 绑定分身，服务器登记路由。

## 软件侧真实工程量（硬件之外的大头）

1. **ffmpeg MPP 版自建**：现 manifest 的 johnvansickle 静态版是纯
   软编（A72 软编 1080p ~0.5× 实时还发热），本地化必须自建带
   rkmpp 硬编的 ffmpeg（Rockchip 官方补丁分支）——agpkg Tier3 +1。
2. **音频前端**：扬声器放 TTS 同时 mic 要听见打断 → AEC + VAD +
   波束（speexdsp/webrtc-audio-processing 可跑），调参依赖腔体/
   增益/双 mic 布局，ODM 联调项，NRE 合同必须单列。
3. **表情渲染**：framebuffer 直绘（灯/眼睛动画状态机）。文字栈
   定档：**ab_glyph**（纯 Rust OTF 栅格化，零 C 依赖）+ **NotoSansCJK
   子集**（几百字+ASCII，几百 KB；最终架构无 Android 层，
   /system/fonts 那课作废，字体自己带进 rootfs，全量字体只给
   typst/ffmpeg）+ qrcode crate 出码 + 自写 CJK 断行（不需要
   cosmic-text）。坑：Noto 官方分发是 .otc 合集须确认 collection
   index 支持或子集化时拆单；ab_glyph 无 hinting，按物理像素/2 倍
   分辨率栅格化即够，不引 fontdue
4. **开机绑定流程 + 设备路由登记**（咬合 relay）。
5. **记忆备份**：rclone 定期推服务器（单点失忆对冲）。

视觉通路 v1 定调：本地拍照 → AginxBrain 路由云端 vision API；
本地小 vision 模型（Qwen-VL 2B int4 在 6TOPS 勉强）v2 再议。

## 数据面扩展备忘（watch item，非依赖）

OpenDuck（[CITGuru/openduck](https://github.com/CITGuru/openduck)，
2026-09 调研）：开源版 MotherDuck——`ATTACH 'openduck:mydb'` 挂
远程库、单查询 LOCAL/REMOTE 双端执行、差分存储。**569★ 单人项目
已休眠（停更 5 月起），不进依赖不进 agpkg**。记它的原因：卡上
8GB RAM 跑不动全量历史分析（session-events 长期积累/跨分身记忆
挖掘）是真实缺口，「重活上移」目前只答了 ffmpeg/LLM，数据查询类
重活无答案——dual execution 是这个问题的漂亮形态（agent 写 SQL
不用知道数据在哪）。真到 8GB 查不动那天，第一动作仍是朴素解：
大表放服务器 PG/duckdb，卡上 agent 远程查询；MotherDuck 模式有
成熟开源实现时再评估升级。

## 已拍板 / 待拍板

已拍板：五官俱全「有点人样」、v1 按需唤醒 + v2 常听、secure boot
进 v1、SoC RK3576、WiFi-only（无人用 SIM）、摄像头走局部凸台。

待拍板（不阻塞阶段 0）：

1. 首发配置档确认（¥500 全配 vs 极简档）——倾向全配
2. 听筒去留（贴耳场景是否真实存在）
3. 阶段 1 白牌基型选哪家方案商（RFQ 三家）

## 与生态的咬合

- agpkg（20 件已镜像）是这台设备的「手」的肌腱——见
  [AGPKG-CANDIDATES.md](AGPKG-CANDIDATES.md)
- Termux 对照研究的三主课在自产机语境下更新：字体问题**直接消解**
  （两层都是我们的，boot 挂载即可，见 [TERMUX-STUDY.md](TERMUX-STUDY.md)）；
  recipes 仓与分发 fallback 仍然有效
- 商业闭环：分身进化 → 分享活实例 → 卖肉身（本卡）→ 远程使用收费
- 竞品参照：[VIOLOOP-STUDY.md](VIOLOOP-STUDY.md)——RK3576 与 BYOK
  双撞款验证（亿元融资竞品同选型同架构）；carrier 侧五条可学清单
