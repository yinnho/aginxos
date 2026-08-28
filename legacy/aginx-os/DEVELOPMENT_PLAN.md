# Aginx OS 开发计划 (2026-04-02)

## 目标
最终目标：Pixel 5手机正常使用Aginx OS
- 内核正常启动到shell
- 显示"Aginx OS"文字
- 基本输入输出（UART调试）
- 网络连接能力
- 可安装到手机并重启使用

## 当前状态

**已完成**：
- ✅ QEMU内核基础（UART、MMU、内存管理）
- ✅ TCP/IP网络栈（DHCP、ping、TCP连接）
- ✅ 任务调度器（多任务支持）
- ✅ Shell命令系统（14个命令）

**主要问题**：
1. **Pixel 5无法启动**：最小内核（仅WFI循环）也导致"error boot prepare"
2. **QEMU构建需要修复**：刚修复了链接器错误
3. **硬件支持不完整**：缺乏Pixel 5的引导、显示、输入驱动

## 核心原则

**真机优先**：Pixel 5引导是最大风险，必须尽早攻克。如果引导无解，后续所有硬件驱动都是空谈。
**QEMU辅助**：QEMU用于功能开发和快速验证，不作为最终目标。
**用户空间延后**：EL0隔离对"手机能正常使用"不是前置条件，先让内核在真机上跑起来。

## 工作计划（五个阶段）

### 第一阶段：QEMU验证 + Pixel 5引导攻坚（并行，1-3周）

#### 1A: QEMU环境验证与修复
**目标**：确保QEMU内核功能完整，作为真机开发的参照

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 1A.1 | 修复QEMU构建错误 | ✅ 已完成 | 高 |
| 1A.2 | 验证QEMU所有命令：help、clear、halt、reboot、tasks、spawn、dhclient、status、ping、listen、connect、sendmsg、mem、uptime | 待开始 | 高 |
| 1A.3 | 解决QEMU 10.x MMU/GIC兼容性问题 | 待开始 | 中 |

**验证方法**：
```bash
# 构建并启动QEMU
cargo build --release
qemu-system-aarch64 -machine virt,gic-version=3,highmem=off -cpu cortex-a57 \
  -kernel target/aarch64-unknown-none/release/aginx-kernel -nographic \
  -netdev user,id=net0,hostfwd=tcp::5555-:8080 -device virtio-net-device,netdev=net0
# 依次测试每个命令
```

#### 1B: Pixel 5引导攻坚（最高优先级）
**目标**：解决"error boot prepare"，让内核在真机上执行到入口点

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 1B.1 | 从设备提取官方boot.img，分析其格式 | 待开始 | 高 |
| 1B.2 | 研究ABL验证机制：是否必须签名，locked/unlocked差异 | 待开始 | 高 |
| 1B.3 | 用mkbootimg或Google工具重建boot.img | 待开始 | 高 |
| 1B.4 | 获取ABL串口日志（fastboot oem dmesg 或串口线） | 待开始 | 高 |
| 1B.5 | 尝试不同加载地址（0x80000000 / 0x80080000 / 0x00000000） | 待开始 | 中 |
| 1B.6 | 从设备提取DTB，分析内存映射和保留区域 | 待开始 | 中 |

**已知线索**：
- `fastboot boot` 返回 "Volume Corrupt" — boot镜像格式肯定有问题
- 最小内核（仅WFI）同样失败 — 不是内核代码的问题
- 设备已解锁 — 验证不是主要障碍

**关键问题待解答**：
1. ABL期望什么格式的boot.img？（v0.9/v1/v2/v3/v4？）
2. kernel_addr / ramdisk_addr / tags_addr 应该设什么值？
3. 是否需要DTB附加在boot.img中？
4. pagesize应该是4096还是2048？

### 第二阶段：Pixel 5最小可用系统（2-4周）
**目标**：真机上显示文字 + 响应shell命令
**前置条件**：第一阶段1B完成（内核能在真机执行）

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 2.1 | Framebuffer初始化 + 填充颜色测试 | 待开始 | 高 |
| 2.2 | 字体渲染（8x16 bitmap font）| 待开始 | 高 |
| 2.3 | 显示"Aginx OS"文字（任务#34） | 待开始 | 高 |
| 2.4 | QUP UART输出（shell提示符） | 待开始 | 高 |
| 2.5 | QUP UART输入（接收命令） | 待开始 | 高 |
| 2.6 | 基本shell命令在真机运行 | 待开始 | 高 |
| 2.7 | PSCI重启/关机 | 待开始 | 中 |

**成功标准**：
- 屏幕显示"Aginx OS"及shell提示符
- 通过UART能输入命令并看到输出
- 能正常重启和关机

### 第三阶段：Pixel 5硬件驱动（3-4周）
**目标**：实现可交互的手机系统
**前置条件**：第二阶段完成

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 3.1 | 触摸屏或物理按键输入驱动 | 待开始 | 高 |
| 3.2 | eMMC存储驱动 + 简单文件系统 | 待开始 | 中 |
| 3.3 | 电源管理（PSCI完善，休眠/唤醒） | 待开始 | 中 |
| 3.4 | WiFi驱动（ath11k QCA6390） | 待开始 | 低 |

### 第四阶段：系统集成（2-3周）
**目标**：可重复刷机，稳定运行

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 4.1 | 创建刷机包（支持A/B分区，备份原始系统） | 待开始 | 高 |
| 4.2 | 启动流程优化（logo、进度提示） | 待开始 | 中 |
| 4.3 | 稳定性测试（长时间运行） | 待开始 | 中 |
| 4.4 | 恢复机制（出错可回到Android） | 待开始 | 高 |

### 第五阶段：完善与扩展（持续）
**目标**：从"能跑"到"能用"

| 任务 | 描述 | 状态 | 优先级 |
|------|------|------|--------|
| 5.1 | 用户空间支持（EL0 + syscall） | 待开始 | 中 |
| 5.2 | Scheme/IPC层（Redox风格） | 待开始 | 低 |
| 5.3 | SSH远程访问 | 待开始 | 低 |
| 5.4 | 基本应用 | 待开始 | 低 |
| 5.5 | Agent运行时 | 待开始 | 远期 |

## 里程碑定义

- **里程碑1**：QEMU验证通过 + Pixel 5内核执行到入口点（UART有输出或framebuffer有变化）
- **里程碑2**：Pixel 5显示"Aginx OS"文字 + shell响应命令（任务#34完成）
- **里程碑3**：Pixel 5可交互使用（输入+输出+存储）
- **里程碑4**：手机正常重启，系统保持稳定，可恢复到Android

## 风险与缓解

| 风险 | 影响 | 概率 | 缓解方案 |
|------|------|------|----------|
| ABL要求签名，解锁仍不可绕过 | 第一阶段1B失败 | 中 | 研究custom AVB key；或换用支持lk2nd的设备（如旧Qualcomm手机） |
| 内存映射冲突，内核地址不可用 | 第一阶段1B延迟 | 中 | 从DTB分析可用内存区域，调整链接地址 |
| QUP UART未初始化，无法调试 | 第二阶段受阻 | 低 | 先用framebuffer输出调试信息 |
| WiFi驱动过于复杂 | 第三阶段跳过 | 高 | 延后，先实现有线网络或USB网络 |

## 立即行动

### 今天（2026-04-02）— 已完成
1. [x] 修复QEMU构建错误（linker.ld符号问题）
2. [x] 验证QEMU环境：所有子系统正常（需用`virtio-net-pci`）
3. [x] 分析Pixel 5 boot.img格式
4. [x] 发现create_boot.py的V3头部完全错误（用了V0/V1/V2字段布局）
5. [x] 用AOSP mkbootimg重建正确的V3 boot.img
6. [x] 恢复设备到正常状态（stock factory image）

### 关键发现（2026-04-02）
1. **boot.img格式问题已修复**：create_boot.py混合V0-V2和V3格式，用AOSP mkbootimg替代
2. **Pixel 5 GKI架构**：boot(V3) = kernel + ramdisk, vendor_boot(V3) = vendor ramdisk + DTB + cmdline
3. **内核仍无法引导**：即使用正确的V3 boot.img + stock ramdisk，设备仍返回fastboot
4. **内核可能的问题**：
   - 链接地址不正确（当前0x80080000，可能需要0x80000000+0x8000）
   - 缺少正确的设备树初始化
   - 需要串口日志定位具体崩溃原因
5. **QEMU修复**：entry.S栈指针加载方式修复，FP/SIMD启用

### 下一步（优先级排序）
1. [ ] 研究Pixel 5 ABL的kernel加载地址和入口要求
2. [ ] 获取串口调试日志（USB串口线或`fastboot oem dmesg`）
3. [ ] 尝试修改text_offset为0，kernel_addr为0x10000000
4. [ ] 研究GKI kernel格式要求（LZ4压缩 vs 原始Image）

## 进度跟踪

### 2026-04-02
- **完成**：QEMU环境修复验证、boot.img格式分析、设备恢复
- **问题**：内核在Pixel 5上无法引导，需要串口日志
- **风险**：ABL可能要求特定的内核初始化序列
- **设备状态**：已恢复到stock Android，正常工作

---

*最后更新：2026-04-02*
*计划制定者：Claude Code*
*批准状态：已批准*