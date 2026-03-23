# AgentOS 最小原型设计文档

## 概述

**核心理念**: 一个原生的 Agent Harness 操作系统，以"意图"为调度单位，以"上下文"为稀缺资源。

**目标平台**: Linux 用户态（后续可移植到裸机）
**Agent 形式**: Rust 原生程序
**LLM 支持**: 混合模式（本地小模型 + 远程 API）

---

## 1. 核心抽象

### 1.1 Agent

Agent 不是进程，是一个**有目标的执行单元**：

```rust
struct Agent {
    id: AgentId,
    goal: Goal,           // 要完成的目标（不是代码入口）
    state: AgentState,    // Running | Paused | Blocked | Completed
    context: Context,     // 上下文窗口（受管理的一级资源）
    capabilities: Caps,   // 能力集合（动态授予/撤销）
    memory: MemoryRef,    // 持久化记忆的引用
}
```

### 1.2 Goal（意图）

Goal 是调度的基本单位，不是指令流：

```rust
struct Goal {
    id: GoalId,
    description: String,      // 自然语言描述
    success_criteria: Vec<Test>,  // 验证条件
    priority: Priority,
    deadline: Option<Time>,
    dependencies: Vec<GoalId>, // 依赖的其他 Goal
}
```

### 1.3 Context（上下文）

Context 是**一级资源**，像内存一样被管理：

```rust
struct Context {
    window: Vec<Token>,       // 当前窗口
    max_tokens: usize,        // 上限
    compaction_policy: Policy,// 压缩策略
    working_set: Vec<String>, // 当前工作集（文件、状态等）
}
```

### 1.4 Capability（能力）

不是传统的文件权限，是**语义化的能力描述**：

```rust
enum Capability {
    FileSystem { paths: Vec<PathBuf>, mode: AccessMode },
    Network { domains: Vec<String>, ports: Vec<u16> },
    Execute { commands: Vec<String> },
    SpawnAgent { max_count: usize },
    CallLLM { model: ModelType, rate_limit: RateLimit },
}
```

---

## 2. 系统架构

```
┌─────────────────────────────────────────────────────────┐
│                      CLI Interface                       │
│                   (用户输入 / 输出)                        │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                   Intent Scheduler                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │ Goal Queue  │  │ Scheduler   │  │ Verifier       │  │
│  │ (优先级队列) │  │ (意图调度)  │  │ (完成度验证)    │  │
│  └─────────────┘  └─────────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    Agent Runtime                         │
│  ┌───────────┐  ┌───────────┐  ┌─────────────────────┐  │
│  │ Loader    │  │ Isolator  │  │ Context Manager     │  │
│  │(加载agent)│  │(沙箱隔离)  │  │(上下文压缩/恢复)    │  │
│  └───────────┘  └───────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    Harness Layer                         │
│  ┌───────────┐  ┌───────────┐  ┌─────────────────────┐  │
│  │ Tool Bus  │  │ Memory Mgr│  │ State Persist       │  │
│  │(工具调用) │  │(记忆管理)  │  │(状态持久化)         │  │
│  └───────────┘  └───────────┘  └─────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    LLM Gateway                           │
│  ┌───────────────────┐  ┌───────────────────────────┐   │
│  │ Local Engine      │  │ Remote API               │   │
│  │ (llama.cpp 等)    │  │ (OpenAI, Anthropic 等)   │   │
│  └───────────────────┘  └───────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────┐
│                    Linux Kernel                          │
│              (进程、内存、文件系统、网络)                   │
└─────────────────────────────────────────────────────────┘
```

---

## 3. 核心组件设计

### 3.1 Intent Scheduler（意图调度器）

**职责**: 调度 Goal，不是调度 CPU 时间片

```rust
impl IntentScheduler {
    // 核心调度循环
    fn run(&mut self) {
        loop {
            // 1. 选择最高优先级的 Goal
            let goal = self.goal_queue.pop_highest_priority();

            // 2. 检查依赖是否满足
            if !self.dependencies_satisfied(&goal) {
                self.goal_queue.requeue(goal);
                continue;
            }

            // 3. 选择或创建 Agent 来执行
            let agent = self.select_agent(&goal);

            // 4. 执行直到完成或需要切换
            loop {
                let result = agent.step();

                match result {
                    StepResult::Progress => continue,
                    StepResult::Blocked => {
                        self.goal_queue.requeue(goal);
                        break;
                    }
                    StepResult::Completed => {
                        // 5. 验证是否真正完成
                        if self.verify(&goal) {
                            self.mark_completed(&goal);
                        } else {
                            self.retry_or_escalate(&goal);
                        }
                        break;
                    }
                    StepResult::Failed(e) => {
                        self.handle_failure(&goal, e);
                        break;
                    }
                }
            }
        }
    }
}
```

### 3.2 Context Manager（上下文管理器）

**职责**: 管理 token 窗口，自动压缩和恢复

```rust
impl ContextManager {
    // 当上下文快满时自动压缩
    fn maybe_compact(&mut self, agent: &mut Agent) {
        if self.usage_ratio() > 0.8 {
            // 1. 提取关键信息
            let summary = self.summarize(&agent.context);

            // 2. 保存完整上下文到记忆
            self.memory.store_full_context(&agent.id, &agent.context);

            // 3. 用摘要替换
            agent.context = Context::from_summary(summary);
        }
    }

    // 当 Agent 需要之前的上下文时恢复
    fn restore(&mut self, agent: &mut Agent, query: &str) {
        // 从记忆中检索相关片段
        let relevant = self.memory.retrieve_relevant(&agent.id, query);
        agent.context.prepend(relevant);
    }
}
```

### 3.3 Tool Bus（工具总线）

**职责**: 管理 Agent 的能力调用

```rust
impl ToolBus {
    fn execute(&mut self, agent: &Agent, tool_call: ToolCall) -> Result<ToolResult> {
        // 1. 检查权限
        if !agent.capabilities.allows(&tool_call) {
            return Err(Error::CapabilityDenied);
        }

        // 2. 在沙箱中执行
        let result = self.sandbox.execute(tool_call)?;

        // 3. 记录审计日志
        self.audit_log.record(&agent.id, &tool_call, &result);

        // 4. 返回结果
        Ok(result)
    }
}
```

### 3.4 Memory Manager（记忆管理器）

**职责**: 持久化 Agent 的记忆

```rust
struct MemoryEntry {
    timestamp: DateTime,
    event_type: EventType,
    content: String,
    embedding: Vec<f32>,  // 用于语义检索
}

impl MemoryManager {
    fn store(&mut self, agent_id: &AgentId, entry: MemoryEntry) {
        // 存储到持久化存储（SQLite / 文件）
        self.db.insert(agent_id, entry);
    }

    fn retrieve_relevant(&self, agent_id: &AgentId, query: &str, k: usize) -> Vec<MemoryEntry> {
        // 语义检索最相关的 k 条记忆
        let query_embedding = self.embed(query);
        self.db.vector_search(agent_id, &query_embedding, k)
    }
}
```

---

## 4. 目录结构

```
agentos/
├── Cargo.toml
├── src/
│   ├── main.rs                 # CLI 入口
│   ├── lib.rs
│   │
│   ├── core/
│   │   ├── mod.rs
│   │   ├── agent.rs            # Agent 定义
│   │   ├── goal.rs             # Goal 定义
│   │   ├── context.rs          # Context 管理
│   │   └── capability.rs       # Capability 系统
│   │
│   ├── scheduler/
│   │   ├── mod.rs
│   │   ├── intent.rs           # 意图调度器
│   │   └── verifier.rs         # 完成度验证
│   │
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── loader.rs           # Agent 加载器
│   │   ├── isolator.rs         # 沙箱隔离（namespaces, seccomp）
│   │   └── ipc.rs              # Agent 间通信
│   │
│   ├── harness/
│   │   ├── mod.rs
│   │   ├── tool_bus.rs         # 工具总线
│   │   ├── memory.rs           # 记忆管理
│   │   └── state.rs            # 状态持久化
│   │
│   ├── llm/
│   │   ├── mod.rs
│   │   ├── gateway.rs          # LLM 网关
│   │   ├── local.rs            # 本地模型（llama.cpp binding）
│   │   └── remote.rs           # 远程 API
│   │
│   └── cli/
│       ├── mod.rs
│       └── repl.rs             # 交互式 CLI
│
├── agents/                      # 内置 Agent 程序
│   ├── shell_agent/
│   └── coder_agent/
│
└── tests/
    └── integration/
```

---

## 5. 最小 MVP 范围

第一阶段只实现核心闭环：

| 组件 | MVP 范围 |
|------|---------|
| Intent Scheduler | 简单的 FIFO 队列 + 单 Goal 执行 |
| Context Manager | 固定窗口，无自动压缩 |
| Tool Bus | 3 个工具：Read, Write, Execute |
| Memory | 文件存储，无向量检索 |
| LLM Gateway | 只支持 OpenAI API |
| Isolator | 无隔离，直接在主进程运行 |

**MVP 目标**: 用户输入一个 Goal，系统调用 LLM 规划，执行工具调用，完成目标。

---

## 6. 实现步骤

### Phase 1: 核心骨架 (1-2 周)
1. 项目结构搭建
2. Agent/Goal/Context 类型定义
3. 简单的 CLI 入口

### Phase 2: LLM 集成 (1 周)
4. OpenAI API 客户端
5. Tool calling 集成
6. 基本的 Agent 执行循环

### Phase 3: Harness 层 (1-2 周)
7. Tool Bus 实现（Read/Write/Execute）
8. 文件系统记忆存储
9. 状态持久化

### Phase 4: 调度器 (1 周)
10. 多 Goal 队列
11. 优先级调度
12. 简单的完成度验证

---

## 7. 验证方式

MVP 完成后，应该能运行：

```bash
# 启动 AgentOS
$ agentos

# 输入 Goal
> Create a hello world program in Python and run it

# AgentOS 自动：
# 1. 调用 LLM 规划步骤
# 2. 创建 hello.py
# 3. 执行 python hello.py
# 4. 验证输出包含 "Hello, World!"
# 5. 报告完成
```

---

## 决策记录

- **项目名称**: AgentOS
- **存储后端**: SQLite（支持 sqlite-vec 扩展做向量检索）
- **MVP 隔离**: 不需要（主进程内运行，后续版本再加）
