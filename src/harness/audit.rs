use crate::core::AgentId;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// 审计日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub agent_id: AgentId,
    pub event_type: AuditEventType,
    pub tool_name: Option<String>,
    pub arguments: Option<String>,
    pub result: Option<String>,
    pub success: bool,
    pub duration_ms: Option<u64>,
}

/// 审计事件类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    ToolCall,
    ToolResult,
    LlmRequest,
    LlmResponse,
    GoalStart,
    GoalComplete,
    Error,
}

impl AuditEventType {
    pub fn as_str(&self) -> &str {
        match self {
            AuditEventType::ToolCall => "tool_call",
            AuditEventType::ToolResult => "tool_result",
            AuditEventType::LlmRequest => "llm_request",
            AuditEventType::LlmResponse => "llm_response",
            AuditEventType::GoalStart => "goal_start",
            AuditEventType::GoalComplete => "goal_complete",
            AuditEventType::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "tool_call" => AuditEventType::ToolCall,
            "tool_result" => AuditEventType::ToolResult,
            "llm_request" => AuditEventType::LlmRequest,
            "llm_response" => AuditEventType::LlmResponse,
            "goal_start" => AuditEventType::GoalStart,
            "goal_complete" => AuditEventType::GoalComplete,
            _ => AuditEventType::Error,
        }
    }
}

/// 审计日志管理器
pub struct AuditLog {
    db: Connection,
}

impl AuditLog {
    /// 创建新的审计日志（持久化到文件）
    pub fn new(path: &str) -> Result<Self, crate::core::Error> {
        let db = Connection::open(path).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to open audit log: {}", e))
        })?;

        Self::init_tables(&db)?;

        Ok(Self { db })
    }

    /// 创建内存中的审计日志
    pub fn in_memory() -> Result<Self, crate::core::Error> {
        let db = Connection::open_in_memory().map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create in-memory audit log: {}", e))
        })?;

        Self::init_tables(&db)?;

        Ok(Self { db })
    }

    fn init_tables(db: &Connection) -> Result<(), crate::core::Error> {
        db.execute(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                tool_name TEXT,
                arguments TEXT,
                result TEXT,
                success INTEGER NOT NULL,
                duration_ms INTEGER
            )",
            [],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create audit table: {}", e))
        })?;

        // 创建索引以便快速查询
        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_agent ON audit_log(agent_id)",
            [],
        ).ok();

        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp)",
            [],
        ).ok();

        Ok(())
    }

    /// 记录工具调用
    pub fn log_tool_call(
        &self,
        agent_id: &AgentId,
        tool_name: &str,
        arguments: &serde_json::Value,
        result: &str,
        success: bool,
        duration_ms: u64,
    ) -> Result<i64, crate::core::Error> {
        let timestamp = Utc::now().to_rfc3339();

        self.db.execute(
            "INSERT INTO audit_log (timestamp, agent_id, event_type, tool_name, arguments, result, success, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                timestamp,
                agent_id.to_string(),
                AuditEventType::ToolCall.as_str(),
                tool_name,
                arguments.to_string(),
                result,
                success as i32,
                duration_ms as i64,
            ],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to log tool call: {}", e))
        })?;

        Ok(self.db.last_insert_rowid())
    }

    /// 记录通用事件
    pub fn log_event(
        &self,
        agent_id: &AgentId,
        event_type: AuditEventType,
        details: &str,
    ) -> Result<i64, crate::core::Error> {
        let timestamp = Utc::now().to_rfc3339();

        self.db.execute(
            "INSERT INTO audit_log (timestamp, agent_id, event_type, result, success)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![
                timestamp,
                agent_id.to_string(),
                event_type.as_str(),
                details,
            ],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to log event: {}", e))
        })?;

        Ok(self.db.last_insert_rowid())
    }

    /// 查询 Agent 的审计日志
    pub fn query_by_agent(
        &self,
        agent_id: &AgentId,
        limit: usize,
    ) -> Result<Vec<AuditEntry>, crate::core::Error> {
        let mut stmt = self.db
            .prepare(
                "SELECT id, timestamp, agent_id, event_type, tool_name, arguments, result, success, duration_ms
                 FROM audit_log
                 WHERE agent_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare query: {}", e))
            })?;

        let entries = stmt
            .query_map(params![agent_id.to_string(), limit as i64], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    agent_id: row.get::<_, String>(2)?.parse()
                        .unwrap_or_default(),
                    event_type: AuditEventType::from_str(&row.get::<_, String>(3)?),
                    tool_name: row.get(4)?,
                    arguments: row.get(5)?,
                    result: row.get(6)?,
                    success: row.get::<_, i32>(7)? != 0,
                    duration_ms: row.get::<_, Option<i64>>(8)?.map(|ms| ms as u64),
                })
            })
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to query audit log: {}", e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to parse audit entries: {}", e))
            })?;

        Ok(entries)
    }

    /// 获取工具调用统计
    pub fn tool_stats(&self, agent_id: &AgentId) -> Result<ToolStats, crate::core::Error> {
        let mut stmt = self.db
            .prepare(
                "SELECT tool_name, COUNT(*) as count, SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as success_count
                 FROM audit_log
                 WHERE agent_id = ?1 AND event_type = 'tool_call'
                 GROUP BY tool_name",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare stats query: {}", e))
            })?;

        let mut stats = ToolStats::default();

        let rows = stmt
            .query_map(params![agent_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to query stats: {}", e))
            })?;

        for row in rows {
            let (tool_name, count, success_count) = row.map_err(|e| {
                crate::core::Error::Internal(format!("Failed to parse stats row: {}", e))
            })?;
            stats.total_calls += count;
            stats.successful_calls += success_count;
            stats.by_tool.insert(tool_name, ToolCallStats { calls: count, successes: success_count });
        }

        Ok(stats)
    }

    /// 清理旧日志
    pub fn prune_old(&self, keep_days: u32) -> Result<usize, crate::core::Error> {
        let cutoff = Utc::now() - chrono::Duration::days(keep_days as i64);

        let deleted = self.db.execute(
            "DELETE FROM audit_log WHERE timestamp < ?1",
            params![cutoff.to_rfc3339()],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to prune audit log: {}", e))
        })?;

        Ok(deleted)
    }
}

/// 工具调用统计
#[derive(Debug, Default)]
pub struct ToolStats {
    pub total_calls: u64,
    pub successful_calls: u64,
    pub by_tool: std::collections::HashMap<String, ToolCallStats>,
}

impl ToolStats {
    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.successful_calls as f64 / self.total_calls as f64
        }
    }
}

/// 单个工具的调用统计
#[derive(Debug, Clone)]
pub struct ToolCallStats {
    pub calls: u64,
    pub successes: u64,
}
