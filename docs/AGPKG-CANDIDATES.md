# agpkg 候选册（2026-08-30 调研 + 实测剥壳；08-31 增补媒体/文档管线 4 件）

CLI 工具生态普查产物：20 个工具已从上游 release 下载、剥壳、逐个
`file` 验证（ELF aarch64）并算出**裸二进制 sha256**。上游资产全是
压缩包而 agpkg 装裸文件——按 codex/grok 先例走剥壳镜像：把这批
`.bin` 原字节上传到 `yinnho/aginxos` releases（tag `<name>-<ver>`，
资产名 `<name>-<ver>-aarch64-unknown-linux-musl`），下方 manifest
行的 sha256 即刻生效（镜像必须原字节，改一字节就失配）。

**镜像已建**：20/20 releases 全部上传成功（2026-08-31），manifest
行可按需启用；上机后逐个跑 `--version` 是最终验证闸门
（aginxbrowser v0.2.5 先例：静态 ELF 仍可能真机 segfault）。

## Tier 1+2 manifest 行（镜像后即用）

```
# --- 上游官方 musl aarch64（Tier 1）---
opencode  https://github.com/yinnho/aginxos/releases/download/opencode-v1.18.25/opencode-v1.18.25-aarch64-unknown-linux-musl df6ac02ded2beb634375c18872dec40e007bfb27c9f5a7274656c4d13a8281e7
ripgrep   https://github.com/yinnho/aginxos/releases/download/ripgrep-15.2.0/ripgrep-15.2.0-aarch64-unknown-linux-musl c14cdb389f34e504d69e386cfc67d5c5d9a730a990de03ca6910b2a15e30386a
fd        https://github.com/yinnho/aginxos/releases/download/fd-v10.5.0/fd-v10.5.0-aarch64-unknown-linux-musl 90dab774d92889926d75a85b47c4b2dc4c9adfa792cd3a6ccfcb98b0eabc9b94
bat       https://github.com/yinnho/aginxos/releases/download/bat-v0.26.1/bat-v0.26.1-aarch64-unknown-linux-musl ba7c30e56a3de25fead14f9d2fa8bdde9dd974a118e367e0de6748c7322a4ae8
zoxide    https://github.com/yinnho/aginxos/releases/download/zoxide-v0.10.0/zoxide-v0.10.0-aarch64-unknown-linux-musl 1864ef686a652b9db90901c72dbb8bd3da45d98738d12f94b4f85729448f2dde
duckdb    https://github.com/yinnho/aginxos/releases/download/duckdb-v1.5.5/duckdb-v1.5.5-aarch64-unknown-linux-musl f4b0a66a8166fe45aa1fefb1b0be86c0d6222690022a0816e4adf353795d45c2
yazi      https://github.com/yinnho/aginxos/releases/download/yazi-v26.8.15/yazi-v26.8.15-aarch64-unknown-linux-musl d5d0eee9322cbc67432be492d3e725370de5951495c2d69556d919f427b128d5
bottom    https://github.com/yinnho/aginxos/releases/download/bottom-0.14.9/bottom-0.14.9-aarch64-unknown-linux-musl db127978c2e0a32d35e53f084d36ba937b1fe261bec27a6974c8f9f5d83ef0cb

# --- Go 静态（无 musl 名，静态链接 musl 兼容；Tier 2）---
crush     https://github.com/yinnho/aginxos/releases/download/crush-v0.91.2/crush-v0.91.2-aarch64-unknown-linux-musl 4b8814416643fc291c261f4de8d5e581bf9667a7b8c594a274bcd769b1bec7e6
fzf       https://github.com/yinnho/aginxos/releases/download/fzf-v0.74.3/fzf-v0.74.3-aarch64-unknown-linux-musl 0b8520ae96426c592feffa1171c704d66f8db6f4ed28f0d238fcd65a5628bfff
gojq      https://github.com/yinnho/aginxos/releases/download/gojq-v0.12.19/gojq-v0.12.19-aarch64-unknown-linux-musl 2f8ae7d8204f8b4cf5960186a89357e2c5e71bf644de02e1120a46c1a4dc6d40
miller    https://github.com/yinnho/aginxos/releases/download/miller-v6.21.0/miller-v6.21.0-aarch64-unknown-linux-musl 2508708146e479f5171ea65a84b2d812294757ba70c1e67449f6196fe17feaf8
rclone    https://github.com/yinnho/aginxos/releases/download/rclone-v1.75.0/rclone-v1.75.0-aarch64-unknown-linux-musl a7094d6e48c6c26cb069175ae93ee221db7dabfa18f57cb6bf3d3d5e1fb1cf3a
lazygit   https://github.com/yinnho/aginxos/releases/download/lazygit-v0.64.1/lazygit-v0.64.1-aarch64-unknown-linux-musl a726ea89fb026b99b1237ac23f8aa227c8c7fa903f9715131ae7d92eb7a2bea7
glow      https://github.com/yinnho/aginxos/releases/download/glow-v3.0.0/glow-v3.0.0-aarch64-unknown-linux-musl 6399cbf0277be0d5125a0c18d2830e419557c3090fed4f6fa3c8253413c2e9f4

# --- 媒体/文档管线（08-31 增补，全静态）---
ffmpeg    https://github.com/yinnho/aginxos/releases/download/ffmpeg-v7.0.2/ffmpeg-v7.0.2-aarch64-unknown-linux-musl 6bb182d0d75d23028db82e9e4f723ca69b853d055698486e6984ddb2c06fb8ce
ffprobe   https://github.com/yinnho/aginxos/releases/download/ffprobe-v7.0.2/ffprobe-v7.0.2-aarch64-unknown-linux-musl d17ae9b4c297d48e2521ba14e417bb0537c6ff77c584cdbcd6bb0d8d0307a2e8
typst     https://github.com/yinnho/aginxos/releases/download/typst-v0.15.1/typst-v0.15.1-aarch64-unknown-linux-musl 3088dd985a891d804a98c69db24dfca77a35878e45d40e38c79cf36d72bcd4c1
pandoc    https://github.com/yinnho/aginxos/releases/download/pandoc-3.11/pandoc-3.11-aarch64-unknown-linux-musl 80e7b7b04282e6fb5dfd245c6be5957c204a398aece6935b43c9f3ac1fe38dff
lux       https://github.com/yinnho/aginxos/releases/download/lux-v0.24.1/lux-v0.24.1-aarch64-unknown-linux-musl c1f1bb63ef40d25729be5cd699204823de310c0e57dc0676d6ff96841c4cbce7

# --- 自建档（自有仓 release，同 aginx 形制，无需镜像）---
xlsx      https://github.com/yinnho/xlsx/releases/download/v0.1.0/xlsx-aarch64-unknown-linux-musl 78a6bf454b453a68880d00ea860d0725fa2cf137dedb62a4526a1580fc1f083f
```

## 镜像上传操作（一条循环跑完）

剥壳产物在 `/tmp/agpkg-staging/*.bin`（macOS /tmp 三天自动清，过期按
provenance 表重下、用上游 sha256 验后再传）。上传=在 `yinnho/aginxos`
为每个工具建独立 release，资产名按 manifest URL 命名：

```bash
items=(
  "opencode|opencode-v1.18.25|sst/opencode opencode-linux-arm64-musl.tar.gz"
  "ripgrep|ripgrep-15.2.0|BurntSushi/ripgrep ripgrep-15.2.0-aarch64-unknown-linux-musl.tar.gz"
  "fd|fd-v10.5.0|sharkdp/fd fd-v10.5.0-aarch64-unknown-linux-musl.tar.gz"
  "bat|bat-v0.26.1|sharkdp/bat bat-v0.26.1-aarch64-unknown-linux-musl.tar.gz"
  "zoxide|zoxide-v0.10.0|ajeetdsouza/zoxide zoxide-0.10.0-aarch64-unknown-linux-musl.tar.gz"
  "duckdb|duckdb-v1.5.5|duckdb/duckdb duckdb_cli-linux-arm64-musl.gz"
  "yazi|yazi-v26.8.15|sxyazi/yazi yazi-aarch64-unknown-linux-musl.zip"
  "bottom|bottom-0.14.9|ClementTsang/bottom bottom_aarch64-unknown-linux-musl.tar.gz"
  "crush|crush-v0.91.2|charmbracelet/crush crush_0.91.2_Linux_arm64.tar.gz"
  "fzf|fzf-v0.74.3|junegunn/fzf fzf-0.74.3-linux_arm64.tar.gz"
  "gojq|gojq-v0.12.19|itchyny/gojq gojq_v0.12.19_linux_arm64.tar.gz"
  "miller|miller-v6.21.0|johnkerl/miller miller-6.21.0-linux-arm64.tar.gz"
  "rclone|rclone-v1.75.0|rclone/rclone rclone-v1.75.0-linux-arm64.zip"
  "lazygit|lazygit-v0.64.1|jesseduffield/lazygit lazygit_0.64.1_linux_arm64.tar.gz"
  "glow|glow-v3.0.0|charmbracelet/glow glow_3.0.0_Linux_arm64.tar.gz"
  "ffmpeg|ffmpeg-v7.0.2|johnvansickle.com/ffmpeg ffmpeg-release-arm64-static.tar.xz"
  "ffprobe|ffprobe-v7.0.2|johnvansickle.com/ffmpeg ffmpeg-release-arm64-static.tar.xz"
  "typst|typst-v0.15.1|typst/typst typst-aarch64-unknown-linux-musl.tar.xz"
  "pandoc|pandoc-3.11|jgm/pandoc pandoc-3.11-linux-arm64.tar.gz"
)
for it in "${items[@]}"; do
  IFS='|' read -r name tag src <<<"$it"
  gh release create "$tag" -R yinnho/aginxos \
    --title "$tag musl (official, repacked)" \
    --notes "Raw binary repacked from $src (upstream sha256 pinned in docs/AGPKG-CANDIDATES.md)" \
    "/tmp/agpkg-staging/$name.bin#$tag-aarch64-unknown-linux-musl"
done
```

传完抽查：`gh release view <tag> -R yinnho/aginxos --json assets` 的
digest 应等于 manifest 行的 sha256（GitHub 存的就是文件 sha256）。

## 上游钉档（provenance，镜像���必依赖但留档）

| 工具 | 上游版本 | 上游资产 | 上游 sha256 |
|---|---|---|---|
| opencode | v1.18.25 | sst/opencode …/opencode-linux-arm64-musl.tar.gz | e9144dca…c2f52de0 |
| ripgrep | 15.2.0 | BurntSushi/ripgrep …/ripgrep-15.2.0-aarch64-unknown-linux-musl.tar.gz | 800b1e72…e1740915 |
| fd | v10.5.0 | sharkdp/fd …/fd-v10.5.0-aarch64-unknown-linux-musl.tar.gz | d76c4317…92c374d4 |
| bat | v0.26.1 | sharkdp/bat …/bat-v0.26.1-aarch64-unknown-linux-musl.tar.gz | 6369242c…96c2c23 |
| zoxide | v0.10.0 | ajeetdsouza/zoxide …/zoxide-0.10.0-aarch64-unknown-linux-musl.tar.gz | f1f16c5d…3ef641 |
| duckdb | v1.5.5 | duckdb/duckdb …/duckdb_cli-linux-arm64-musl.gz | b4967d32…84df764 |
| yazi | v26.8.15 | sxyazi/yazi …/yazi-aarch64-unknown-linux-musl.zip | dfaafadd…908cab4b |
| bottom | 0.14.9 | ClementTsang/bottom …/bottom_aarch64-unknown-linux-musl.tar.gz | 7b6d532f…66125533 |
| crush | v0.91.2 | charmbracelet/crush …/crush_0.91.2_Linux_arm64.tar.gz | b9eb8179…b6d64a2 |
| fzf | v0.74.3 | junegunn/fzf …/fzf-0.74.3-linux_arm64.tar.gz | 4a17a17b…60463046 |
| gojq | v0.12.19 | itchyny/gojq …/gojq_v0.12.19_linux_arm64.tar.gz | b22794b4…623d8b5 |
| miller | v6.21.0 | johnkerl/miller …/miller-6.21.0-linux-arm64.tar.gz | 3f4bc159…de467d27 |
| rclone | v1.75.0 | rclone/rclone …/rclone-v1.75.0-linux-arm64.zip | d0ad88ba…ccfa5203 |
| lazygit | v0.64.1 | jesseduffield/lazygit …/lazygit_0.64.1_linux_arm64.tar.gz | 8b7ca3b4…bb8b6e7 |
| glow | v3.0.0 | charmbracelet/glow …/glow_3.0.0_Linux_arm64.tar.gz | 810c39f4…47a99759 |
| ffmpeg | 7.0.2 | johnvansickle.com …/ffmpeg-release-arm64-static.tar.xz | f4149bb2…7201b1（构建 2024-06，站方更新慢但全静态）|
| ffprobe | 7.0.2 | 同上（同 tarball）| 同上 |
| typst | v0.15.1 | typst/typst …/typst-aarch64-unknown-linux-musl.tar.xz | 5aa8d74a…23dcee |
| pandoc | 3.11 | jgm/pandoc …/pandoc-3.11-linux-arm64.tar.gz | 56ed5566…314c79a |
| lux | v0.24.1 | iawia002/lux …/lux_0.24.1_Linux_arm64.tar.gz | 479b0929…873879d7 |

## 链接形态备注（上机验证项）

- **完全静态**：ripgrep/fd/bat/zoxide/yazi/bottom/crush/fzf/gojq/miller/rclone/lazygit/glow —— 13 个，musl 裸环境直接跑
- **musl 动态**：opencode、duckdb —— `interpreter /lib/ld-musl-aarch64.so.1`，musl 原生根文件系统（本机）自带 loader，可跑；若根文件系统精简掉 musl libc 则需换静态自建
- **glibc 工具链全静态**：ffmpeg、ffprobe（johnvansickle 构建，"statically linked, for GNU/Linux 3.7.0"）、pandoc（GHC 静态）——无 interpreter 无动态库，musl 根文件系统直接跑
- yazi zip 内还有伴生 `ya` 守护进程（未入册；yazi 本体单跑可用）

## Tier 3 — 上游无 aarch64 musl，需自建（未入册）

| 工具 | 现状 | 自建路径 |
|---|---|---|
| eza | 只有 gnu（含 no_libgit 变体） | `cargo-zigbuild --target aarch64-unknown-linux-musl`，用 no_libgit 特性树 |
| qsv | 22.0.1 只有 gnu zip | 同上（qsv 曾发 musl，新版掉了；自建注意 `--no-default-features` 减依赖） |
| pdftotext(poppler)/newsboat | C/C++，无官方静态 musl | 交叉编译成本高，按需再议 |
| whisper.cpp | 官方只发 ubuntu-arm64（glibc） | CI alpine 容器交叉出 musl 静态（ggml 纯 C++）；**附带模型分发**（ggml.bin 57~466MB，agpkg 需非 bin 资产装法） |
| resvg | 只发 linux-x86_64 | Rust zigbuild 半小时；typst 出 PNG 已覆盖大半，按需 |
| alass（字幕对轴） | 只发 x86_64 | Rust zigbuild；小众按需 |

## 视频制作配套缺口（08-31 全景盘点）

主干已闭环：ffmpeg（转码/拼接/抽帧/tile 接触表/GIF/loudnorm/libass 烧字幕）
+ ffprobe（探针/质检）+ typst（海报字幕卡出 PNG）+ lux（B站/yt 下载）+
rclone（分发）。真缺口两个：

1. **中文字体包**——libass 烧字幕与 typst 排版都要 CJK 字体，musl
   rootfs 裸奔=豆腐块。Noto Sans CJK ~20MB/字重。这是 **agpkg 格式
   缺口**：manifest 现只装裸 bin，字体要装 fonts 目录且 fontconfig
   可见——需 assets 扩展（加装目录列或独立 manifest），属格式立法。
2. **whisper.cpp musl 自建 + 模型放置约定**（见 Tier3 表）——补齐后
   视频字幕自动生成 + 语音消息转文字一次到位。

非本地件：Seedance/Seedream 生成与 TTS 配音走服务端 API；NLE 不做
（agent 视频是程序化组装，ffmpeg filter_complex 即剪辑 DSL）。

## 与生态的咬合点

- **自由档扩容**：opencode/crush 接 codex/grok 之后——都是本机 session 存储，适合 aclone 两级结构同构改造
- **agent 数据面**：qsv（自建后）/duckdb/gojq/miller 给化身 shell_allow 面——CSV/JSON 零 python 处理
- **aterm app 生态**：lazygit/yazi/bottom/glow 是按需启动 TUI，不占常驻内存
- **文档生成套装 CLI 化**（08-31 定向）：opencarrier 的 document_generate 本就是 pandoc 薄壳——AginxOS 上直接装 pandoc+typst 即得 md→docx/pptx + pdf 管线（`pandoc in.md -o out.typ && typst compile`；pandoc 直出 pdf 需 LaTeX 引擎，机上没有）。**xlsx 是唯一缺口**：无 pandoc 支持、机型无 python，需新建小 Rust CLI（rust_xlsxwriter 写 + calamine 读，JSON spec 进出）
- **媒体面**：ffmpeg/ffprobe 补齐化身音视频转码/抽帧/probe 能力（Seedance 类产品 flow 上机的硬前置）
