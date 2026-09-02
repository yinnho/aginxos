# 外部观察 Watchlist（2026-09 起）

原则：**只记不依赖**——已休眠/强传染/不同赛道的外部项目，各留
一句「是什么 + 为什么记 + 触发再评估的条件」。

## OpenDuck——数据面的 MotherDuck 形态

[CITGuru/openduck](https://github.com/CITGuru/openduck)（569★，
2026-05 起休眠，MIT）：`ATTACH 'openduck:mydb'` 挂远程库、单查询
LOCAL/REMOTE 双端执行、差分存储（append-only 层+快照读）。

**记它**：卡上 8GB RAM 跑不动全量历史分析（session-events 积累/
跨分身记忆挖掘）是真实缺口，「重活上移」只答了 ffmpeg/LLM，
数据查询类重活无答案——dual execution 是这个问题的漂亮形态
（agent 写 SQL 不用知道数据在哪）。

**再评估触发**：项目复活；或 MotherDuck 模式出现成熟开源实现。
在那之前的第一动作是朴素解：大表放服务器 PG/duckdb，卡上远程查。

## TabTin——人+多 Agent 团队协作平台

[tabtin-ai/TabTin](https://github.com/tabtin-ai/TabTin)（237★，
2026-08 开源，Public Preview，**AGPL-3.0 强传染——只看思路不动
代码**）：任务交接（冻结上下文+有权限的文件引用）、人机共编
文档/表格、Agent 角色带规则/模型/Skill/记忆、治理三件套。

**记它**：①架构第三次撞款——「移动端无独立执行环境，配合
电脑 daemon」= 界面设备≠执行设备，与我们的「人手机=微信界面、
卡=身体」同构（前两次：Violoop RK3576、BYOK，见
[VIOLOOP-STUDY.md](VIOLOOP-STUDY.md)）；②**任务交接包**概念正中
我们链式 cron 流水线痛点——现在 output/<pipeline_id>/状态.md
台账是贫民版，升级方向=「上下文快照+文件引用+权限检查」正式化
成接续包，cron 链接续/分身间转交/人中途接管吃同一格式。

**再评估触发**：链式 cron 管线下次迭代（把交接包正式化时参考
其冻结-引用-权限三件结构）；或其 Community Server 生态成势。

## AGIROS——具身智能 OS 的社区标准赛道（生态雷达）

[agiros.org.cn](https://agiros.org.cn/)（开放原子/openEuler 体系，
月报 2026-08）：ROS 生态机器人 OS 社区——DDS 中间件、物理仿真、
WAM→轻量 VLA 决策迁移、平台×硬件兼容性认证。

**记它**：①「兼容性认证」模式（认证申请→收录→选型参考）是
agpkg 包×设备验证矩阵未来的呈现层模板；②WAM→VLA「迁移决策
机制而非最终动作」与 Violoop 权重蒸馏、我们 flow/validator
同构——第三家印证「通用智能越来越便宜，专用能力才需要沉淀」，
我们的蒸馏层是显式数据（flow/validator/clone 知识），比权重
黑盒可控；③具身 OS 的社区标准赛道存在且在组织化（世界机器人
大会/人才认证），agent 硬件品类起来后可能有标准位窗口。

**再评估触发**：agpkg 装机量上来需要兼容性呈现层；或考虑
社区/标准位站位时。

## 附：已完成研究（不在 watchlist，已转行动）

- Termux → [TERMUX-STUDY.md](TERMUX-STUDY.md)（recipes 仓/fallback
  已进行动项）
- OpenLogi → [OPENLOGI-STUDY.md](OPENLOGI-STUDY.md)（Apache-2.0
  可白嫖，periph 立项参考）
- Violoop → [VIOLOOP-STUDY.md](VIOLOOP-STUDY.md)（carrier 五条
  可学清单）
