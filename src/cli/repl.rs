use crate::core::*;
use crate::harness::{MemoryManager, StateManager};
use crate::llm::LlmGateway;
use crate::runtime::AgentExecutor;
use crate::scheduler::IntentScheduler;
use nu_ansi_term::Color;
use reedline::{DefaultPrompt, Reedline, Signal};

/// CLI REPL
pub struct Repl {
    editor: Reedline,
    prompt: DefaultPrompt,
    gateway: LlmGateway,
    scheduler: IntentScheduler,
    state: StateManager,
}

impl Repl {
    pub fn new() -> Result<Self, Error> {
        let editor = Reedline::create();
        let prompt = DefaultPrompt::default();

        // 从环境变量获取 API key
        let api_key = std::env::var("OPENAI_API_KEY").ok();
        let model = std::env::var("AGENTOS_MODEL").ok();

        let gateway = LlmGateway::new(api_key, model);
        let scheduler = IntentScheduler::new();
        let state = StateManager::new(".agentos/state.json");

        Ok(Self {
            editor,
            prompt,
            gateway,
            scheduler,
            state,
        })
    }

    /// 运行 REPL
    pub async fn run(&mut self) -> Result<(), Error> {
        self.print_welcome();

        loop {
            let signal = self.editor.read_line(&self.prompt);

            match signal {
                Ok(Signal::Success(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    // 处理内置命令
                    if self.handle_builtin_command(line).await {
                        continue;
                    }

                    // 作为 Goal 处理
                    self.process_goal(line).await?;
                }
                Ok(Signal::CtrlC) => {
                    println!("\nGoodbye!");
                    break;
                }
                Ok(Signal::CtrlD) => {
                    println!("\nGoodbye!");
                    break;
                }
                Ok(Signal::CtrlL) => {
                    // 清屏 - 重新打印欢迎信息
                    self.print_welcome();
                }
                Err(err) => {
                    eprintln!("Error: {:?}", err);
                    break;
                }
            }
        }

        // 保存状态
        self.state.save()?;
        Ok(())
    }

    fn print_welcome(&self) {
        println!(
            "{}",
            Color::Cyan.paint(r#"
   ___                  ____  _____
  / _ | ____  ___  ___ / __ \/ ___/
 / __ |/ __ \/ _ \/ _ // /_/ / /
/_/ |_/_/ /_/\___/\_,_/\____/_/
"#)
        );
        println!("{}", Color::Green.paint("  Agent Harness Operating System"));
        println!();
        println!("Type your goal and press Enter. Type :help for commands.");
        println!();
    }

    /// 处理内置命令
    async fn handle_builtin_command(&mut self, line: &str) -> bool {
        match line {
            ":help" => {
                println!("Commands:");
                println!("  :help     - Show this help");
                println!("  :status   - Show system status");
                println!("  :goals    - Show pending goals");
                println!("  :quit     - Exit AgentOS");
                println!();
                println!("Or type any goal in natural language.");
                true
            }
            ":quit" | ":exit" => {
                println!("Goodbye!");
                std::process::exit(0);
            }
            ":status" => {
                let state = self.state.state();
                println!("Version: {}", state.version);
                println!("Agents: {}", state.agents.len());
                println!("Pending: {}", state.pending_goals.len());
                println!("Completed: {}", state.completed_goals.len());
                true
            }
            ":goals" => {
                let state = self.state.state();
                println!("Pending Goals:");
                for goal in &state.pending_goals {
                    println!("  - {} [{:?}]", goal.description, goal.priority);
                }
                if state.pending_goals.is_empty() {
                    println!("  (none)");
                }
                true
            }
            _ => false,
        }
    }

    /// 处理 Goal
    async fn process_goal(&mut self, description: &str) -> Result<(), Error> {
        println!("{}", Color::Yellow.paint(format!("\n→ Goal: {}", description)));
        println!();

        // 创建 Goal
        let goal = Goal::new(description);

        // 创建 Agent
        let mut agent = Agent::new("default");
        agent.grant_capability(Capability::FileSystem {
            paths: vec![std::env::current_dir().unwrap_or_default()],
            mode: AccessMode::ReadWrite,
        });
        agent.grant_capability(Capability::Execute {
            commands: vec!["*".into()], // MVP: 允许所有命令
        });

        // 注册到状态
        self.state.register_agent(&agent);
        self.state.add_pending_goal(goal.clone());

        // 创建执行器
        let memory = MemoryManager::in_memory()?;
        let scheduler = IntentScheduler::new();
        let mut executor = AgentExecutor::new(
            agent,
            LlmGateway::new(
                std::env::var("OPENAI_API_KEY").ok(),
                std::env::var("AGENTOS_MODEL").ok(),
            ),
            memory,
            scheduler,
        );

        // 执行
        print!("{}", Color::Blue.paint("Working"));
        let result = executor.run(goal.clone()).await;

        match result {
            Ok(StepResult::Completed) => {
                println!("{}", Color::Green.paint("\n✓ Goal completed!"));
                self.state.complete_goal(goal.id);
            }
            Ok(StepResult::Failed(e)) => {
                println!("{}", Color::Red.paint(format!("\n✗ Failed: {}", e)));
            }
            Ok(other) => {
                println!("{:?}", other);
            }
            Err(e) => {
                println!("{}", Color::Red.paint(format!("\n✗ Error: {}", e)));
            }
        }

        println!();
        Ok(())
    }
}
