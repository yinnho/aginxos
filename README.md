# AgentOS

An Agent Harness Operating System that schedules **goals** instead of processes and manages **context** as a first-class resource.

## Concept

```
Traditional OS          →    AgentOS
─────────────────────────────────────────
Process                 →    Goal (intent-based scheduling)
CPU time slices         →    Context window (token management)
System calls            →    Tool calls
File permissions        →    Capabilities (semantic access control)
Memory                  →    Persistent memory (SQLite)
```

**Formula**: `Model + Harness = Agent`

The Harness handles:
- Tool execution
- Context management (compaction, restoration)
- Memory persistence
- Capability enforcement
- Verification loops

## Architecture

```
┌─────────────────────────────────────────┐
│              CLI Interface               │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│           Intent Scheduler               │
│   Goal Queue │ Scheduler │ Verifier     │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│           Agent Runtime                  │
│   Loader │ Context Manager │ Isolator   │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│           Harness Layer                  │
│   Tool Bus │ Memory Mgr │ State Persist │
└─────────────────────────────────────────┘
                    │
                    ▼
┌─────────────────────────────────────────┐
│           LLM Gateway                    │
│   Local (llama.cpp) │ Remote (OpenAI)   │
└─────────────────────────────────────────┘
```

## Installation

```bash
git clone https://github.com/yinnho/agentos.git
cd agentos
cargo build --release
```

## Usage

### Interactive Mode

```bash
export OPENAI_API_KEY=your_key
./target/release/agentos
```

```
   ___                  ____  _____
  / _ | ____  ___  ___ / __ \/ ___/
 / __ |/ __ \/ _ \/ _ // /_/ / /
/_/ |_/_/ /_/\___/\_,_/\____/_/

  Agent Harness Operating System

Type your goal and press Enter. Type :help for commands.

> Create a hello world program in Python and run it
→ Goal: Create a hello world program in Python and run it

Working...
✓ Goal completed!
```

### Non-Interactive Mode

```bash
./target/release/agentos "List all Rust files in the project"
```

### CLI Commands

| Command | Description |
|---------|-------------|
| `:help` | Show available commands |
| `:status` | Show system status |
| `:goals` | Show pending goals |
| `:quit` | Exit AgentOS |

## Core Abstractions

### Agent

An execution unit with a goal, not a code entry point:

```rust
struct Agent {
    id: AgentId,
    goal: Goal,           // What to accomplish
    state: AgentState,    // Running | Paused | Blocked | Completed
    context: Context,     // Managed context window
    capabilities: Vec<Capability>,
}
```

### Goal

The scheduling unit - intent, not instruction stream:

```rust
struct Goal {
    id: GoalId,
    description: String,
    success_criteria: Vec<Test>,
    priority: Priority,
    dependencies: Vec<GoalId>,
}
```

### Capability

Semantic access control, not file permissions:

```rust
enum Capability {
    FileSystem { paths: Vec<PathBuf>, mode: AccessMode },
    Network { domains: Vec<String>, ports: Vec<u16> },
    Execute { commands: Vec<String> },
    CallLLM { model: String, max_tokens: usize },
}
```

### Context

Token window as a managed resource:

```rust
struct Context {
    messages: Vec<Message>,
    max_tokens: usize,
    current_tokens: usize,
    compaction_policy: CompactionPolicy,
}
```

## Built-in Tools

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents |
| `write_file` | Write content to file |
| `execute` | Run shell commands |

## Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `OPENAI_API_KEY` | OpenAI API key |
| `AGENTOS_MODEL` | Model to use (default: gpt-4o / llama3.2) |
| `AGENTOS_LLM_PROVIDER` | Provider: `openai`, `ollama`, `custom` |
| `OLLAMA_BASE_URL` | Ollama server URL (default: http://localhost:11434/v1) |
| `AGENTOS_LLM_BASE_URL` | Custom LLM API base URL |
| `AGENTOS_LLM_API_KEY` | Custom LLM API key |

### Using Local LLM (Ollama)

1. Install and start Ollama:
```bash
# macOS/Linux
curl -fsSL https://ollama.com/install.sh | sh
ollama serve
```

2. Pull a model:
```bash
ollama pull llama3.2
# or
ollama pull qwen2.5
ollama pull deepseek-r1
```

3. Run AgentOS:
```bash
# Auto-detect Ollama
./target/release/agentos

# Or explicitly set
export AGENTOS_LLM_PROVIDER=ollama
export AGENTOS_MODEL=llama3.2
./target/release/agentos
```

### Using Custom LLM Endpoint

```bash
export AGENTOS_LLM_PROVIDER=custom
export AGENTOS_LLM_BASE_URL=http://your-llm-server:8000/v1
export AGENTOS_LLM_API_KEY=your_key_if_needed
export AGENTOS_MODEL=your-model-name
./target/release/agentos
```

## Development

```bash
# Run tests
cargo test

# Run with debug logging
RUST_LOG=debug cargo run
```

## Roadmap

- [x] Local LLM support (Ollama)
- [x] Context compaction with LLM summarization
- [ ] Vector-based memory retrieval (sqlite-vec)
- [ ] Multi-agent coordination
- [ ] Agent isolation (namespaces/seccomp)
- [ ] Goal verification framework
- [ ] Web UI

## License

MIT

## Inspiration

- [Anthropic: Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents)
- [Redox OS](https://www.redox-os.org/)
