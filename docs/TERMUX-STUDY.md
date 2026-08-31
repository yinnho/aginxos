# Termux 套路对照研究（2026-08-31）

调研对象：[termux org](https://github.com/termux)（termux-app 60k★，
termux-packages 16.8k★，干了十年，1200+ 包）。核心问题：**和我们
（AginxOS = 手机 + musl rootfs + agent）是不是同构？能学到什么？**

结论：同构的是「把完整 Unix 生态塞进手机」这件事本身。我们接下来
要踩的坑他们几乎全踩过、全解决过。但要分清哪些能抄、哪些是他们
「无 root 寄居 Android」的特产税、我们住在自家 rootfs 根本没有。

对照基线：[AGPKG-CANDIDATES.md](AGPKG-CANDIDATES.md)（20 件已镜像）。

## 一、他们是什么形态（事实）

- **termux-app**（终端模拟器）+ **termux-packages**（构建系统 monorepo）
  双核。包不是二进制仓库，是 **recipes 仓**：每包一个目录，一个
  `build.sh`：

  ```bash
  TERMUX_PKG_HOMEPAGE=…
  TERMUX_PKG_DESCRIPTION="…"
  TERMUX_PKG_LICENSE="MIT"
  TERMUX_PKG_VERSION=2.0.0
  TERMUX_PKG_REVISION=4
  TERMUX_PKG_SRCURL="https://github.com/…/v$TERMUX_PKG_VERSION.tar.gz"
  TERMUX_PKG_SHA256=ce88f92c…      # 钉上游源码包内容
  TERMUX_PKG_DEPENDS="ffmpeg"
  TERMUX_PKG_AUTO_UPDATE=true      # CI 自动跟上游发新版

  termux_step_pre_configure() { … }   # 生命周期钩子函数
  termux_step_make_install() { … }
  ```

- 规模：`packages/` 1000+ 目录、`x11-packages/` 941 个（GitHub API
  单页上限即截断，实际更多）。
- 分发：自建 **apt 仓**（Hetzner 服务器 + Cloudflare CDN 赞助
  packages-cf.termux.dev）。历史教训：F-Droid 慢 + GitHub 在部分地区
  不可达，是被逼出来的自治镜像体系。
- 质量闸门：**构建即测试**——包在目标环境（Android/Bionic）里从源码
  编出来，能构建 ≈ 能跑；另有 repology 元数据自动跟踪版本新旧。
- 周边：termux-elf-cleaner（剥 ELF section 消 Bionic 警告）、
  termux-exec（execve 包装修 shebang）、command-not-found（敲错命令
  推荐该装什么）。

## 二、能直接学的（按对我们的价值排序）

### 1. 字体问题：他们根本没打包 CJK 字体（对我们最大的意外发现）

Termux 连一个 CJK 字体包都没有。fontconfig 配方就两行：

```bash
--with-default-fonts=/system/fonts
--with-add-fonts=$TERMUX_PREFIX/share/fonts
```

**直接用 Android 系统字体**——每台安卓机出厂就带 NotoSansCJK。
推论：如果 AginxOS 的 musl rootfs 跑在 Android 之上，烧字幕
（libass）/typst 排版要的中文字体**可能根本不需要 agpkg 立法什么
assets 格式**——boot 时把 `/system/fonts` 挂进（或拷进）rootfs +
fontconfig 指过去就完了。

→ 行动项：候选册里「字体 = agpkg 格式缺口」的拍板项，**第一动作
是查 rootfs 能不能看见 /system/fonts**，而不是改 manifest 格式。
若不可见再谈打包（届时打 NotoSansCJK 单包，几十 MB，agpkg 需扩展
非 bin 资产装法）。

### 2. recipes 仓是核心资产——我们该建 aginx-packages

我们现在：xlsx 一套 release workflow、agc 一套、Tier3 的
eza/qsv/whisper.cpp/resvg/alass 每个再来一套 = 不可持续。Termux 的
答案：**包定义（recipe）和包产物（二进制）分离，recipe 收敛进一个
monorepo 统一 CI 构建**。

→ 行动项：建 `aginx-packages` 仓。每包一个 recipe（上游 URL +
sha256 + 构建方式：zigbuild-rust / go-static / alpine-cpp /
mirror-剥壳），一个 workflow 全矩阵出 musl 资产。现状资产收编：
- xlsx、agc 的现有 workflow → 头两个配方
- 20 件剥壳镜像（AGPKG-CANDIDATES.md）→ mirror 类配方（recipe 记
  上游 provenance + 剥壳规则，升级时重跑）
- 顺手：**alass 在 termux-packages 里有现成 recipe**（Rust +
  depends ffmpeg），抄来改成 musl 目标即得一个 Tier3 件。

### 3. 分发可靠性：镜像/CDN 是生死线

我们 manifest 全部指 GitHub releases 直链。Termux 的教训摆在那：
GitHub 在部分地区不可达是常态。我们的手机是微信生态的中国大陆
设备——**装机第一次痛快 ≠ 重装/新机/恢复时痛快**（本机到 GitHub
CDN 都已断流，curl exit 56/28）。

→ 行动项（三选一或组合）：manifest 行支持 fallback URL；或统一
过 ghproxy 类前缀；或 86quan 自建反代（`dl.aginx.net` → GitHub
release 302/代理）。至少备一手，别等装机潮踩坑。

### 4. 版本跟踪：升级语义

manifest 钉 sha256 = 只有安装没有升级，也回答不了「哪些行过期了」。
Termux 有 AUTO_UPDATE + repology。我们不用抄 apt 全套，抄半件事：
**定时 CI 比对上游 release tag → 候选册出 diff 报告**，人工决定
是否重镜像。

### 5. 构建即测试 → 我们镜像路径的等价闸门

他们从源码构建于目标环境，天然可信。我们是镜像上游字节：sha256
对 ≠ 能跑（aginxbrowser 先例：静态 ELF 真机 segfault）。

→ 行动项：CI 里 qemu-aarch64 + musl rootfs 跑每个镜像的
`--version`，**不用真机把最终闸门前移**。镜像进 manifest 前过
这门。

## 三、明确不抄的（他们交的税我们没有）

| 他们的东西 | 为什么不抄 |
|---|---|
| apt/dpkg 依赖解析 | 全静态裸 bin、零依赖是我们的特性（agent 环境永不进 dependency hell）。他们是动态链接 + 依赖树，复杂度大爆炸。 |
| glibc-packages / proot-distro | 无 root 寄居的逃生通道，重到反面教材。我们 rootfs 是自己的地盘。 |
| Bionic 补丁地狱、elf-cleaner、termux-exec | 「寄居 Android 用户态」的税。我们不寄居。 |
| 1200 包的规模 | 他们服务人类终端用户的长尾；agent 手机只要 agent 的手，20~50 个精挑的封顶（候选册方向正确）。 |

## 四、一个更高的观察：包发现 UX

Termux 有 command-not-found（敲错命令推荐装什么）——人类终端的
包发现。我们的对应物：**agent 直接读 AGPKG-CANDIDATES.md 自己决定
装什么**。人是终端 + 包管理器；agent 的 UX 是文档 + manifest。

→ 推论：候选册值得升格成**正式包目录**（名字 / 一句话用途 /
何时用 / manifest 行）——它不是给人看的附录，是 agent 的一手
消费物。格式应按「agent 查得快、读得懂」优化。

## 五、行动项汇总（待拍板）

1. **查 rootfs 可见 /system/fonts 与否**——消解字体拍板项（最小成本
   动作，先查再立法）
2. **aginx-packages recipes 仓立项**——收编 xlsx/agc/20 镜像管线，
   Tier3 自建件（eza/qsv/whisper.cpp/resvg/alass）全走它
3. **分发 fallback**——ghproxy / 自建反代 / manifest 双 URL，三选
4. （次级）上游版本跟踪 CI、镜像 qemu 烟测 CI、候选册升格包目录
