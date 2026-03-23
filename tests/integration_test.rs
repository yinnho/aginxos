use agentos::core::*;
use agentos::scheduler::IntentScheduler;
use agentos::harness::{ToolBus, MemoryManager, EventType};

#[test]
fn test_goal_creation() {
    let goal = Goal::new("Write a hello world program")
        .with_priority(Priority::High);

    assert_eq!(goal.description, "Write a hello world program");
    assert_eq!(goal.priority, Priority::High);
}

#[test]
fn test_agent_creation() {
    let agent = Agent::new("test_agent");

    assert_eq!(agent.name, "test_agent");
    assert_eq!(agent.state, AgentState::Idle);
}

#[test]
fn test_capability_system() {
    let mut agent = Agent::new("test");

    // 授予文件系统权限
    agent.grant_capability(Capability::FileSystem {
        paths: vec!["/tmp".into()],
        mode: AccessMode::ReadWrite,
    });

    // 检查权限
    let cap_read = Capability::FileSystem {
        paths: vec!["/tmp/test.txt".into()],
        mode: AccessMode::ReadOnly,
    };
    assert!(agent.has_capability(&cap_read));

    let cap_forbidden = Capability::FileSystem {
        paths: vec!["/etc/passwd".into()],
        mode: AccessMode::ReadOnly,
    };
    assert!(!agent.has_capability(&cap_forbidden));
}

#[test]
fn test_scheduler() {
    let mut scheduler = IntentScheduler::new();

    let goal1 = Goal::new("Task 1").with_priority(Priority::Low);
    let goal2 = Goal::new("Task 2").with_priority(Priority::High);

    scheduler.enqueue(goal1);
    scheduler.enqueue(goal2);

    // 高优先级应该先出
    let next = scheduler.pop().unwrap();
    assert_eq!(next.description, "Task 2");
}

#[test]
fn test_context() {
    let mut context = Context::new(1000);

    context.push(Message::system("You are a helpful assistant."));
    context.push(Message::user("Hello!"));

    assert_eq!(context.messages.len(), 2);
    assert!(context.current_tokens > 0);
}

#[test]
fn test_memory() {
    let memory = MemoryManager::in_memory().unwrap();
    let agent_id = AgentId::new();

    memory.store(&agent_id, EventType::UserInput, "Test memory")
        .unwrap();

    let entries = memory.retrieve_recent(&agent_id, 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "Test memory");
}

#[test]
fn test_tool_bus() {
    let tool_bus = ToolBus::new();

    // 检查工具定义
    let tools = tool_bus.to_openai_tools();
    assert!(!tools.is_empty());
    assert!(tools.iter().any(|t| t.function.name == "read_file"));
    assert!(tools.iter().any(|t| t.function.name == "write_file"));
    assert!(tools.iter().any(|t| t.function.name == "execute"));
}
