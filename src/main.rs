use agentos::cli::Repl;
use agentos::core::{Agent, Goal, Capability, AccessMode, StepResult};
use agentos::harness::MemoryManager;
use agentos::llm::LlmGateway;
use agentos::runtime::AgentExecutor;
use agentos::scheduler::IntentScheduler;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    // 初始化日志
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    // 检查命令行参数
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        // 非交互模式：直接执行 goal
        let goal_str = args[1..].join(" ");
        run_goal(&goal_str).await;
    } else {
        // 交互模式：启动 REPL
        let mut repl = match Repl::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to initialize: {}", e);
                std::process::exit(1);
            }
        };

        if let Err(e) = repl.run().await {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run_goal(description: &str) {
    println!("→ Goal: {}", description);

    // 创建 Goal
    let goal = Goal::new(description);

    // 创建 Agent
    let mut agent = Agent::new("default");
    agent.grant_capability(Capability::FileSystem {
        paths: vec![std::env::current_dir().unwrap_or_default()],
        mode: AccessMode::ReadWrite,
    });
    agent.grant_capability(Capability::Execute {
        commands: vec!["*".into()],
    });

    // 创建执行器
    let memory = MemoryManager::in_memory().unwrap();
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
    let result = executor.run(goal).await;

    match result {
        Ok(StepResult::Completed) => {
            println!("✓ Goal completed!");
        }
        Ok(StepResult::Failed(e)) => {
            println!("✗ Failed: {}", e);
        }
        Ok(other) => {
            println!("Result: {:?}", other);
        }
        Err(e) => {
            println!("✗ Error: {}", e);
        }
    }
}
