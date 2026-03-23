use agentos::core::*;
use agentos::harness::{ContextManager, MemoryManager};

#[test]
fn test_context_needs_compaction() {
    let mut context = Context::new(1000); // Small limit for testing

    // Add messages until we exceed 80%
    for i in 0..100 {
        context.push(Message::user(format!("Message {} with some content to make it longer", i)));
    }

    // Should need compaction (over 80% of 1000 tokens)
    assert!(context.needs_compaction());
}

#[test]
fn test_compaction_keep_recent() {
    let mut agent = Agent::new("test");
    agent.context = Context::new(1000);
    agent.context.compaction_policy = CompactionPolicy::KeepRecent(5);

    // Add 10 messages
    for i in 0..10 {
        agent.context.push(Message::user(format!("Message {}", i)));
    }

    assert_eq!(agent.context.messages.len(), 10);

    // Compact
    let mut manager = ContextManager::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(manager.compact(&mut agent)).unwrap();

    assert_eq!(result.messages_removed, 5);
    assert_eq!(agent.context.messages.len(), 5);

    // Should keep the last 5 messages
    assert!(agent.context.messages.iter().any(|m| m.content.contains("Message 9")));
    assert!(!agent.context.messages.iter().any(|m| m.content.contains("Message 0")));
}

#[test]
fn test_compaction_system_and_recent() {
    let mut agent = Agent::new("test");
    agent.context = Context::new(1000);
    agent.context.compaction_policy = CompactionPolicy::SystemAndRecent(3);

    // Add system message
    agent.context.push(Message::system("You are a helpful assistant."));
    // Add user messages
    for i in 0..10 {
        agent.context.push(Message::user(format!("User message {}", i)));
    }

    assert_eq!(agent.context.messages.len(), 11);

    // Compact
    let mut manager = ContextManager::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(manager.compact(&mut agent)).unwrap();

    // Should have: 1 system + 3 recent = 4 messages
    assert_eq!(agent.context.messages.len(), 4);

    // First message should still be system
    assert_eq!(agent.context.messages[0].role, Role::System);

    // Should have recent messages
    assert!(agent.context.messages.iter().any(|m| m.content.contains("User message 9")));
}

#[test]
fn test_compaction_saves_tokens() {
    let mut agent = Agent::new("test");
    agent.context = Context::new(1000);
    agent.context.compaction_policy = CompactionPolicy::KeepRecent(2);

    // Add messages
    for i in 0..20 {
        agent.context.push(Message::user(format!("This is a longer message number {} to test token counting", i)));
    }

    let tokens_before = agent.context.current_tokens;

    // Compact
    let mut manager = ContextManager::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(manager.compact(&mut agent)).unwrap();

    assert!(result.tokens_saved > 0);
    assert!(agent.context.current_tokens < tokens_before);
}

#[test]
fn test_compaction_with_memory() {
    let memory = MemoryManager::in_memory().unwrap();
    let agent_id = AgentId::new();

    let mut agent = Agent::new("test");
    agent.id = agent_id;
    agent.context = Context::new(1000);
    agent.context.compaction_policy = CompactionPolicy::KeepRecent(2);

    // Add messages
    for i in 0..5 {
        agent.context.push(Message::user(format!("Message {}", i)));
    }

    // Compact with memory
    let mut manager = ContextManager::new().with_memory(memory);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _result = rt.block_on(manager.compact(&mut agent)).unwrap();

    // Check archived messages are in memory
    // Note: We'd need to expose the memory to check this properly
    assert_eq!(agent.context.messages.len(), 2);
}

#[test]
fn test_compaction_none_policy() {
    let mut agent = Agent::new("test");
    agent.context = Context::new(1000);
    agent.context.compaction_policy = CompactionPolicy::None;

    // Add messages
    for i in 0..100 {
        agent.context.push(Message::user(format!("Message {}", i)));
    }

    let count_before = agent.context.messages.len();

    // Compact should do nothing
    let mut manager = ContextManager::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(manager.compact(&mut agent)).unwrap();

    assert_eq!(result.messages_removed, 0);
    assert_eq!(result.tokens_saved, 0);
    assert_eq!(agent.context.messages.len(), count_before);
}

#[test]
fn test_maybe_compact_when_not_needed() {
    let mut agent = Agent::new("test");
    agent.context = Context::new(10000); // Large limit

    // Add only a few messages (well under 80%)
    for i in 0..5 {
        agent.context.push(Message::user(format!("Message {}", i)));
    }

    // Should not need compaction
    assert!(!agent.context.needs_compaction());

    // maybe_compact should return None
    let mut manager = ContextManager::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(manager.maybe_compact(&mut agent)).unwrap();

    assert!(result.is_none());
}
