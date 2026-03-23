use crate::core::AgentId;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// 记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: i64,
    pub agent_id: AgentId,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub content: String,
}

/// 事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    /// 用户输入
    UserInput,
    /// Agent 思考/规划
    Thinking,
    /// 工具调用
    ToolCall,
    /// 工具结果
    ToolResult,
    /// 错误
    Error,
    /// 完成状态
    Completion,
    /// 其他
    Other,
}

impl EventType {
    fn as_str(&self) -> &str {
        match self {
            EventType::UserInput => "user_input",
            EventType::Thinking => "thinking",
            EventType::ToolCall => "tool_call",
            EventType::ToolResult => "tool_result",
            EventType::Error => "error",
            EventType::Completion => "completion",
            EventType::Other => "other",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "user_input" => EventType::UserInput,
            "thinking" => EventType::Thinking,
            "tool_call" => EventType::ToolCall,
            "tool_result" => EventType::ToolResult,
            "error" => EventType::Error,
            "completion" => EventType::Completion,
            _ => EventType::Other,
        }
    }
}

/// Memory Manager - 记忆管理器
pub struct MemoryManager {
    db: Connection,
}

impl MemoryManager {
    pub fn new(path: &str) -> Result<Self, crate::core::Error> {
        let db = Connection::open(path).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to open database: {}", e))
        })?;

        // 创建表
        db.execute(
            "CREATE TABLE IF NOT EXISTS memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                event_type TEXT NOT NULL,
                content TEXT NOT NULL
            )",
            [],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create table: {}", e))
        })?;

        // 创建索引
        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_agent_id ON memory(agent_id)",
            [],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create index: {}", e))
        })?;

        Ok(Self { db })
    }

    /// 内存模式（不持久化）
    pub fn in_memory() -> Result<Self, crate::core::Error> {
        let db = Connection::open_in_memory().map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create in-memory database: {}", e))
        })?;

        db.execute(
            "CREATE TABLE IF NOT EXISTS memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                event_type TEXT NOT NULL,
                content TEXT NOT NULL
            )",
            [],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create table: {}", e))
        })?;

        Ok(Self { db })
    }

    /// 存储记忆
    pub fn store(
        &self,
        agent_id: &AgentId,
        event_type: EventType,
        content: &str,
    ) -> Result<i64, crate::core::Error> {
        let timestamp = Utc::now().to_rfc3339();

        self.db
            .execute(
                "INSERT INTO memory (agent_id, timestamp, event_type, content) VALUES (?1, ?2, ?3, ?4)",
                params![agent_id.to_string(), timestamp, event_type.as_str(), content],
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to store memory: {}", e))
            })?;

        Ok(self.db.last_insert_rowid())
    }

    /// 检索 Agent 的最近记忆
    pub fn retrieve_recent(
        &self,
        agent_id: &AgentId,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, crate::core::Error> {
        let mut stmt = self.db
            .prepare(
                "SELECT id, agent_id, timestamp, event_type, content
                 FROM memory
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare statement: {}", e))
            })?;

        let entries = stmt
            .query_map(params![agent_id.to_string(), limit as i64], |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    agent_id: row.get::<_, String>(1)?.parse().map_err(|_| {
                        rusqlite::Error::InvalidQuery
                    })?,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    event_type: EventType::from_str(&row.get::<_, String>(3)?),
                    content: row.get(4)?,
                })
            })
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to retrieve memory: {}", e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to parse memory: {}", e))
            })?;

        Ok(entries)
    }
}
