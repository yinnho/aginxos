use crate::core::AgentId;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use ndarray::Array1;

/// Embedding 向量维度 (OpenAI text-embedding-3-small)
pub const EMBEDDING_DIM: usize = 1536;

/// 记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: i64,
    pub agent_id: AgentId,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
}

/// 事件类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub fn as_str(&self) -> &str {
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

    pub fn parse(s: &str) -> Self {
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

/// Embedding 生成器
#[async_trait::async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, crate::core::Error>;
}

/// OpenAI Embedding 生成器
pub struct OpenAIEmbedding {
    client: async_openai::Client<async_openai::config::OpenAIConfig>,
    model: String,
}

impl OpenAIEmbedding {
    pub fn new(api_key: Option<String>, model: Option<String>) -> Self {
        let config = if let Some(key) = api_key {
            async_openai::config::OpenAIConfig::new().with_api_key(key)
        } else {
            async_openai::config::OpenAIConfig::new()
        };

        Self {
            client: async_openai::Client::with_config(config),
            model: model.unwrap_or_else(|| "text-embedding-3-small".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingGenerator for OpenAIEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, crate::core::Error> {
        use async_openai::types::CreateEmbeddingRequestArgs;

        let request = CreateEmbeddingRequestArgs::default()
            .model(&self.model)
            .input([text])
            .build()
            .map_err(|e| crate::core::Error::Llm(format!("Failed to build embedding request: {}", e)))?;

        let response = self.client
            .embeddings()
            .create(request)
            .await
            .map_err(|e| crate::core::Error::Llm(format!("Embedding API error: {}", e)))?;

        let embedding = response.data.first()
            .ok_or_else(|| crate::core::Error::Llm("No embedding returned".into()))?;

        Ok(embedding.embedding.to_vec())
    }
}

/// Mock Embedding 生成器（用于测试）
pub struct MockEmbedding;

#[async_trait::async_trait]
impl EmbeddingGenerator for MockEmbedding {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, crate::core::Error> {
        // 简单的 mock：基于文本内容生成伪向量
        let mut embedding = vec![0.0f32; EMBEDDING_DIM];
        for (i, c) in text.chars().take(EMBEDDING_DIM).enumerate() {
            embedding[i] = (c as u32) as f32 / 65535.0 - 0.5;
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

/// Memory Manager - 记忆管理器（支持向量检索）
pub struct MemoryManager {
    db: Connection,
    embedding_generator: Option<Box<dyn EmbeddingGenerator>>,
}

impl MemoryManager {
    pub fn new(path: &str) -> Result<Self, crate::core::Error> {
        let db = Connection::open(path).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to open database: {}", e))
        })?;

        Self::init_tables(&db)?;

        Ok(Self {
            db,
            embedding_generator: None,
        })
    }

    pub fn in_memory() -> Result<Self, crate::core::Error> {
        let db = Connection::open_in_memory().map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create in-memory database: {}", e))
        })?;

        Self::init_tables(&db)?;

        Ok(Self {
            db,
            embedding_generator: None,
        })
    }

    fn init_tables(db: &Connection) -> Result<(), crate::core::Error> {
        // 创建主记忆表
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
            crate::core::Error::Internal(format!("Failed to create memory table: {}", e))
        })?;

        // 创建向量表（存储为 BLOB）
        db.execute(
            "CREATE TABLE IF NOT EXISTS embeddings (
                memory_id INTEGER PRIMARY KEY,
                embedding BLOB NOT NULL,
                FOREIGN KEY (memory_id) REFERENCES memory(id)
            )",
            [],
        ).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to create embeddings table: {}", e))
        })?;

        // 创建索引
        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_memory_agent ON memory(agent_id)",
            [],
        ).ok();

        Ok(())
    }

    pub fn with_embedding_generator(mut self, generator: Box<dyn EmbeddingGenerator>) -> Self {
        self.embedding_generator = Some(generator);
        self
    }

    pub fn with_openai_embeddings(api_key: Option<String>, model: Option<String>) -> Self {
        Self::in_memory()
            .expect("Failed to create in-memory database for OpenAI embeddings")
            .with_embedding_generator(Box::new(OpenAIEmbedding::new(api_key, model)))
    }

    pub fn with_mock_embeddings() -> Self {
        Self::in_memory()
            .expect("Failed to create in-memory database for mock embeddings")
            .with_embedding_generator(Box::new(MockEmbedding))
    }

    /// 存储记忆（同步版本，不生成 embedding）
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

    /// 存储记忆并生成 embedding（异步版本）
    pub async fn store_with_embedding(
        &self,
        agent_id: &AgentId,
        event_type: EventType,
        content: &str,
    ) -> Result<i64, crate::core::Error> {
        let id = self.store(agent_id, event_type.clone(), content)?;

        // 生成并存储 embedding
        if let Some(generator) = &self.embedding_generator {
            let embedding = generator.embed(content).await?;
            self.store_embedding(id, &embedding)?;
        }

        Ok(id)
    }

    /// 存储 embedding 向量
    fn store_embedding(&self, memory_id: i64, embedding: &[f32]) -> Result<(), crate::core::Error> {
        let bytes: Vec<u8> = embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        self.db
            .execute(
                "INSERT OR REPLACE INTO embeddings (memory_id, embedding) VALUES (?1, ?2)",
                params![memory_id, bytes],
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to store embedding: {}", e))
            })?;

        Ok(())
    }

    /// 检索 Agent 的最近记忆
    pub fn retrieve_recent(
        &self,
        agent_id: &AgentId,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, crate::core::Error> {
        let mut stmt = self.db
            .prepare(
                "SELECT m.id, m.agent_id, m.timestamp, m.event_type, m.content, e.embedding
                 FROM memory m
                 LEFT JOIN embeddings e ON m.id = e.memory_id
                 WHERE m.agent_id = ?1
                 ORDER BY m.timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare statement: {}", e))
            })?;

        let entries = stmt
            .query_map(params![agent_id.to_string(), limit as i64], |row| {
                let embedding_blob: Option<Vec<u8>> = row.get(5)?;
                let embedding = embedding_blob.map(|bytes| {
                    bytes.chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                        .collect()
                });

                Ok(MemoryEntry {
                    id: row.get(0)?,
                    agent_id: row.get::<_, String>(1)?.parse().map_err(|_| {
                        rusqlite::Error::InvalidQuery
                    })?,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    event_type: EventType::parse(&row.get::<_, String>(3)?),
                    content: row.get(4)?,
                    embedding,
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

    /// 基于向量相似度检索相关记忆
    ///
    /// 优化：使用候选池 + Top-K 选择，避免加载所有记忆
    pub async fn retrieve_similar(
        &self,
        agent_id: &AgentId,
        query: &str,
        k: usize,
    ) -> Result<Vec<(MemoryEntry, f32)>, crate::core::Error> {
        let generator = self.embedding_generator.as_ref().ok_or_else(|| {
            crate::core::Error::Internal("No embedding generator configured".into())
        })?;

        let query_embedding = generator.embed(query).await?;

        // 优化：只查询最近 N 条记忆作为候选池（N = k * 10）
        // 这避免了在大量历史记忆中进行全量扫描
        let candidate_limit = std::cmp::max(k * 10, 100);

        let mut stmt = self.db
            .prepare(
                "SELECT m.id, m.agent_id, m.timestamp, m.event_type, m.content, e.embedding
                 FROM memory m
                 INNER JOIN embeddings e ON m.id = e.memory_id
                 WHERE m.agent_id = ?1
                 ORDER BY m.timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare statement: {}", e))
            })?;

        let entries = stmt
            .query_map(params![agent_id.to_string(), candidate_limit as i64], |row| {
                let embedding_blob: Vec<u8> = row.get(5)?;
                let embedding: Vec<f32> = embedding_blob.chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                Ok(MemoryEntry {
                    id: row.get(0)?,
                    agent_id: row.get::<_, String>(1)?.parse().map_err(|_| {
                        rusqlite::Error::InvalidQuery
                    })?,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    event_type: EventType::parse(&row.get::<_, String>(3)?),
                    content: row.get(4)?,
                    embedding: Some(embedding),
                })
            })
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to retrieve memories: {}", e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to parse memories: {}", e))
            })?;

        // 计算相似度
        let query_vec = Array1::from_vec(query_embedding);

        let mut scored: Vec<(MemoryEntry, f32)> = entries
            .into_iter()
            .filter_map(|entry| {
                let emb = entry.embedding.clone()?;
                let entry_vec = Array1::from_vec(emb);
                let similarity = cosine_similarity(&query_vec, &entry_vec);
                Some((entry, similarity))
            })
            .collect();

        // 按相似度降序排序
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // 返回前 k 个
        Ok(scored.into_iter().take(k).collect())
    }

    /// 混合检索：结合时间排序和语义相似度
    pub async fn retrieve_hybrid(
        &self,
        agent_id: &AgentId,
        query: &str,
        k: usize,
        alpha: f32, // 相似度权重 (0.0-1.0), 时间权重为 1-alpha
    ) -> Result<Vec<(MemoryEntry, f32)>, crate::core::Error> {
        let generator = self.embedding_generator.as_ref().ok_or_else(|| {
            crate::core::Error::Internal("No embedding generator configured".into())
        })?;

        let query_embedding = generator.embed(query).await?;

        // 获取所有记忆
        let mut stmt = self.db
            .prepare(
                "SELECT m.id, m.agent_id, m.timestamp, m.event_type, m.content, e.embedding
                 FROM memory m
                 LEFT JOIN embeddings e ON m.id = e.memory_id
                 WHERE m.agent_id = ?1
                 ORDER BY m.timestamp DESC
                 LIMIT 100",
            )
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to prepare statement: {}", e))
            })?;

        let entries: Vec<MemoryEntry> = stmt
            .query_map(params![agent_id.to_string()], |row| {
                let embedding_blob: Option<Vec<u8>> = row.get(5)?;
                let embedding = embedding_blob.map(|bytes| {
                    bytes.chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                        .collect()
                });

                Ok(MemoryEntry {
                    id: row.get(0)?,
                    agent_id: row.get::<_, String>(1)?.parse().map_err(|_| {
                        rusqlite::Error::InvalidQuery
                    })?,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    event_type: EventType::parse(&row.get::<_, String>(3)?),
                    content: row.get(4)?,
                    embedding,
                })
            })
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to retrieve memories: {}", e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                crate::core::Error::Internal(format!("Failed to parse memories: {}", e))
            })?;

        if entries.is_empty() {
            return Ok(vec![]);
        }

        // 计算归一化时间分数（越新越高）
        let now = Utc::now().timestamp();
        let min_time = entries.iter()
            .map(|e| e.timestamp.timestamp())
            .min()
            .unwrap_or(now);

        let time_range = (now - min_time).max(1) as f32;

        let query_vec = Array1::from_vec(query_embedding);

        let mut scored: Vec<(MemoryEntry, f32)> = entries
            .into_iter()
            .map(|entry| {
                // 时间分数：越新越高 (0-1)
                let time_score = (entry.timestamp.timestamp() - min_time) as f32 / time_range;

                // 相似度分数
                let similarity = entry.embedding.as_ref()
                    .map(|emb| {
                        let entry_vec = Array1::from_vec(emb.clone());
                        cosine_similarity(&query_vec, &entry_vec)
                    })
                    .unwrap_or(0.0);

                // 混合分数
                let score = alpha * similarity + (1.0 - alpha) * time_score;

                (entry, score)
            })
            .collect();

        // 按混合分数排序
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Ok(scored.into_iter().take(k).collect())
    }

    /// 删除旧记忆
    pub fn prune_old(&self, agent_id: &AgentId, keep_count: usize) -> Result<usize, crate::core::Error> {
        // 获取要保留的记忆 ID
        let keep_ids: Vec<i64> = {
            let mut stmt = self.db
                .prepare(
                    "SELECT id FROM memory WHERE agent_id = ?1 ORDER BY timestamp DESC LIMIT ?2"
                )
                .map_err(|e| crate::core::Error::Internal(format!("Failed to prepare: {}", e)))?;

            let mapped = stmt.query_map(params![agent_id.to_string(), keep_count as i64], |row| row.get::<_, i64>(0));
            let ids: Result<Vec<i64>, _> = mapped
                .map_err(|e| crate::core::Error::Internal(format!("Query error: {}", e)))?
                .collect();
            ids.map_err(|e| crate::core::Error::Internal(format!("Failed to get IDs: {}", e)))?
        };

        if keep_ids.is_empty() {
            return Ok(0);
        }

        let placeholders: Vec<String> = keep_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "DELETE FROM memory WHERE agent_id = ? AND id NOT IN ({})",
            placeholders.join(",")
        );

        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(agent_id.to_string())];
        for id in keep_ids {
            params_vec.push(Box::new(id));
        }
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let deleted = self.db.execute(&sql, params_refs.as_slice())
            .map_err(|e| crate::core::Error::Internal(format!("Failed to delete: {}", e)))?;

        // 同时删除孤立的 embeddings
        self.db.execute(
            "DELETE FROM embeddings WHERE memory_id NOT IN (SELECT id FROM memory)",
            [],
        ).ok();

        Ok(deleted)
    }
}

/// 计算余弦相似度
fn cosine_similarity(a: &Array1<f32>, b: &Array1<f32>) -> f32 {
    let dot: f32 = (a * b).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a > 0.0 && norm_b > 0.0 {
        dot / (norm_a * norm_b)
    } else {
        0.0
    }
}
