use agentos::core::AgentId;
use agentos::harness::{MemoryManager, EventType, EmbeddingGenerator};

/// 测试用的 Mock Embedding 生成器
struct TestEmbedding;

#[async_trait::async_trait]
impl EmbeddingGenerator for TestEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, agentos::core::Error> {
        // 简单的词袋模型模拟
        let mut embedding = vec![0.0f32; 1536];

        // 基于关键词设置特定维度的值
        if text.contains("hello") || text.contains("greeting") {
            embedding[0] = 1.0;
        }
        if text.contains("file") || text.contains("read") || text.contains("write") {
            embedding[1] = 1.0;
        }
        if text.contains("error") || text.contains("fail") {
            embedding[2] = 1.0;
        }
        if text.contains("success") || text.contains("complete") {
            embedding[3] = 1.0;
        }
        if text.contains("python") || text.contains("code") {
            embedding[4] = 1.0;
        }

        // 归一化
        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for e in &mut embedding {
                *e /= norm;
            }
        }

        Ok(embedding)
    }
}

#[test]
fn test_memory_store_and_retrieve() {
    let manager = MemoryManager::in_memory().unwrap();
    let agent_id = AgentId::new();

    // 存储几条记忆
    manager.store(&agent_id, EventType::UserInput, "Hello, how are you?").unwrap();
    manager.store(&agent_id, EventType::Thinking, "Thinking about the greeting...").unwrap();
    manager.store(&agent_id, EventType::ToolCall, "read_file called").unwrap();

    // 检索最近记忆
    let entries = manager.retrieve_recent(&agent_id, 10).unwrap();

    assert_eq!(entries.len(), 3);
}

#[tokio::test]
async fn test_store_with_embedding() {
    let manager = MemoryManager::in_memory()
        .unwrap()
        .with_embedding_generator(Box::new(TestEmbedding));

    let agent_id = AgentId::new();

    // 存储带 embedding 的记忆
    let id = manager.store_with_embedding(&agent_id, EventType::UserInput, "Say hello to the user").await.unwrap();
    assert!(id > 0);
}

#[tokio::test]
async fn test_vector_retrieval() {
    let manager = MemoryManager::in_memory()
        .unwrap()
        .with_embedding_generator(Box::new(TestEmbedding));

    let agent_id = AgentId::new();

    // 存储带 embedding 的记忆
    manager.store_with_embedding(&agent_id, EventType::UserInput, "Say hello to the user").await.unwrap();
    manager.store_with_embedding(&agent_id, EventType::ToolCall, "Read the file config.txt").await.unwrap();
    manager.store_with_embedding(&agent_id, EventType::Error, "Failed to write file").await.unwrap();

    // 语义检索
    let results = manager.retrieve_similar(&agent_id, "greeting message", 3).await.unwrap();

    assert!(!results.is_empty());
    // "Say hello" 应该排在前面
    assert!(results[0].0.content.contains("hello"));
}

#[tokio::test]
async fn test_hybrid_retrieval() {
    let manager = MemoryManager::in_memory()
        .unwrap()
        .with_embedding_generator(Box::new(TestEmbedding));

    let agent_id = AgentId::new();

    // 存储多条记忆
    for i in 0..10 {
        manager.store_with_embedding(
            &agent_id,
            EventType::UserInput,
            &format!("Message {} about files", i),
        ).await.unwrap();
    }

    // 混合检索
    let results = manager.retrieve_hybrid(&agent_id, "file operations", 5, 0.5).await.unwrap();

    assert_eq!(results.len(), 5);
}

#[test]
fn test_prune_old_memories() {
    let manager = MemoryManager::in_memory().unwrap();
    let agent_id = AgentId::new();

    // 存储多条记忆
    for i in 0..20 {
        manager.store(&agent_id, EventType::Other, &format!("Memory {}", i)).unwrap();
    }

    // 验证存储了 20 条
    let entries = manager.retrieve_recent(&agent_id, 100).unwrap();
    assert_eq!(entries.len(), 20);

    // 清理，只保留 5 条
    let deleted = manager.prune_old(&agent_id, 5).unwrap();
    assert_eq!(deleted, 15);

    // 验证只保留 5 条最新的
    let entries = manager.retrieve_recent(&agent_id, 100).unwrap();
    assert_eq!(entries.len(), 5);
}

#[test]
fn test_different_agents_isolated() {
    let manager = MemoryManager::in_memory().unwrap();

    let agent1 = AgentId::new();
    let agent2 = AgentId::new();

    // 为两个 agent 存储不同的记忆
    manager.store(&agent1, EventType::UserInput, "Agent 1 message").unwrap();
    manager.store(&agent2, EventType::UserInput, "Agent 2 message").unwrap();

    // Agent 1 只能看到自己的记忆
    let entries1 = manager.retrieve_recent(&agent1, 10).unwrap();
    assert_eq!(entries1.len(), 1);
    assert!(entries1[0].content.contains("Agent 1"));

    // Agent 2 只能看到自己的记忆
    let entries2 = manager.retrieve_recent(&agent2, 10).unwrap();
    assert_eq!(entries2.len(), 1);
    assert!(entries2[0].content.contains("Agent 2"));
}

#[test]
fn test_event_type_conversion() {
    assert_eq!(EventType::UserInput.as_str(), "user_input");
    assert!(matches!(EventType::from_str("tool_call"), EventType::ToolCall));
    assert!(matches!(EventType::from_str("unknown"), EventType::Other));
}
