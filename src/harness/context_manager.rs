use crate::core::*;
use crate::harness::MemoryManager;
use crate::harness::EventType;
use crate::llm::LlmGateway;
use serde::{Deserialize, Serialize};

/// 压缩结果
#[derive(Debug, Clone)]
pub struct CompactionResult {
    /// 被压缩掉的消息数
    pub messages_removed: usize,
    /// 摘要（如果有）
    pub summary: Option<String>,
    /// 节省的 token 数
    pub tokens_saved: usize,
}

/// Context Manager - 上下文管理器
pub struct ContextManager {
    memory: Option<MemoryManager>,
    gateway: Option<LlmGateway>,
}

impl ContextManager {
    pub fn new() -> Self {
        Self {
            memory: None,
            gateway: None,
        }
    }

    pub fn with_memory(mut self, memory: MemoryManager) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn with_gateway(mut self, gateway: LlmGateway) -> Self {
        self.gateway = Some(gateway);
        self
    }

    /// 检查并执行压缩
    pub async fn maybe_compact(
        &mut self,
        agent: &mut Agent,
    ) -> Result<Option<CompactionResult>, Error> {
        if !agent.context.needs_compaction() {
            return Ok(None);
        }

        self.compact(agent).await.map(Some)
    }

    /// 执行压缩
    pub async fn compact(&mut self, agent: &mut Agent) -> Result<CompactionResult, Error> {
        match agent.context.compaction_policy {
            CompactionPolicy::None => Ok(CompactionResult {
                messages_removed: 0,
                summary: None,
                tokens_saved: 0,
            }),
            CompactionPolicy::KeepRecent(n) => {
                self.compact_keep_recent(agent, n)
            }
            CompactionPolicy::SystemAndRecent(n) => {
                self.compact_system_and_recent(agent, n)
            }
            CompactionPolicy::Summarize => {
                self.compact_with_summary(agent).await
            }
        }
    }

    /// 保留最近 N 条消息
    fn compact_keep_recent(&mut self, agent: &mut Agent, n: usize) -> Result<CompactionResult, Error> {
        let total = agent.context.messages.len();
        if total <= n {
            return Ok(CompactionResult {
                messages_removed: 0,
                summary: None,
                tokens_saved: 0,
            });
        }

        let removed: Vec<Message> = agent.context.messages.drain(0..total - n).collect();
        let tokens_saved: usize = removed.iter()
            .map(|m| agent.context.estimate_tokens(&m.content))
            .sum();

        // 保存到记忆
        self.save_archived_messages(&agent.id, &removed)?;

        agent.context.current_tokens = agent.context.current_tokens.saturating_sub(tokens_saved);

        Ok(CompactionResult {
            messages_removed: removed.len(),
            summary: None,
            tokens_saved,
        })
    }

    /// 保留系统消息 + 最近 N 条
    fn compact_system_and_recent(&mut self, agent: &mut Agent, n: usize) -> Result<CompactionResult, Error> {
        let total = agent.context.messages.len();
        if total <= n + 1 {
            return Ok(CompactionResult {
                messages_removed: 0,
                summary: None,
                tokens_saved: 0,
            });
        }

        // 收集系统消息的索引
        let system_indices: Vec<usize> = agent.context.messages.iter()
            .enumerate()
            .filter(|(_, m)| m.role == Role::System)
            .map(|(i, _)| i)
            .collect();

        // 收集最近 N 条非系统消息的索引
        let mut recent_non_system_count = 0;
        let mut recent_indices: Vec<usize> = Vec::new();
        for (i, m) in agent.context.messages.iter().enumerate().rev() {
            if m.role != Role::System {
                recent_indices.push(i);
                recent_non_system_count += 1;
                if recent_non_system_count >= n {
                    break;
                }
            }
        }

        // 要保留的索引
        let mut keep_indices = system_indices;
        keep_indices.extend(recent_indices);
        keep_indices.sort();
        keep_indices.dedup();

        // 分离要保留和要归档的消息
        let mut kept_messages = Vec::new();
        let mut archived_messages = Vec::new();

        for (i, msg) in agent.context.messages.drain(..).enumerate() {
            if keep_indices.contains(&i) {
                kept_messages.push(msg);
            } else {
                archived_messages.push(msg);
            }
        }

        let tokens_saved: usize = archived_messages.iter()
            .map(|m| agent.context.estimate_tokens(&m.content))
            .sum();

        self.save_archived_messages(&agent.id, &archived_messages)?;

        agent.context.messages = kept_messages;
        agent.context.current_tokens = agent.context.current_tokens.saturating_sub(tokens_saved);

        Ok(CompactionResult {
            messages_removed: archived_messages.len(),
            summary: None,
            tokens_saved,
        })
    }

    /// 使用 LLM 生成摘要来压缩
    async fn compact_with_summary(&mut self, agent: &mut Agent) -> Result<CompactionResult, Error> {
        let total = agent.context.messages.len();
        if total <= 4 {
            return Ok(CompactionResult {
                messages_removed: 0,
                summary: None,
                tokens_saved: 0,
            });
        }

        // 收集系统消息
        let system_messages: Vec<Message> = agent.context.messages.iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();

        // 收集最近 2 条非系统消息
        let mut recent_count = 0;
        let mut recent_messages: Vec<Message> = Vec::new();
        for m in agent.context.messages.iter().rev() {
            if m.role != Role::System {
                recent_messages.push(m.clone());
                recent_count += 1;
                if recent_count >= 2 {
                    break;
                }
            }
        }
        recent_messages.reverse();

        // 收集要摘要的消息
        let to_summarize: Vec<Message> = agent.context.messages.iter()
            .filter(|m| {
                let is_system = m.role == Role::System;
                let is_recent = recent_messages.iter().any(|r| {
                    r.role == m.role && r.content == m.content
                });
                !is_system && !is_recent
            })
            .cloned()
            .collect();

        if to_summarize.is_empty() {
            return Ok(CompactionResult {
                messages_removed: 0,
                summary: None,
                tokens_saved: 0,
            });
        }

        // 构建摘要请求
        let mut summary_context = Context::new(4000);
        summary_context.push(Message::system(
            "Summarize the following conversation history concisely. \
             Focus on key decisions, important information, and current progress. \
             Keep the summary under 500 words."
        ));

        let history_text = to_summarize.iter()
            .map(|m| format!("{:?}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        summary_context.push(Message::user(&history_text));

        // 调用 LLM 生成摘要
        let gateway = self.gateway.as_ref().ok_or_else(|| {
            Error::Internal("LLM gateway required for summarization".into())
        })?;
        let response = gateway.chat(&summary_context).await?;

        // 保存原始消息到记忆
        self.save_archived_messages(&agent.id, &to_summarize)?;

        // 计算节省的 token
        let tokens_saved: usize = to_summarize.iter()
            .map(|m| agent.context.estimate_tokens(&m.content))
            .sum();

        let summary_tokens = agent.context.estimate_tokens(&response.content);

        // 重建消息列表
        agent.context.messages = system_messages;
        agent.context.push(Message::system(format!(
            "[Previous context summary]\n{}",
            response.content
        )));
        agent.context.messages.extend(recent_messages);

        // 更新 token 计数
        agent.context.current_tokens = agent.context.current_tokens
            .saturating_sub(tokens_saved)
            .saturating_add(summary_tokens);

        Ok(CompactionResult {
            messages_removed: to_summarize.len(),
            summary: Some(response.content),
            tokens_saved: tokens_saved.saturating_sub(summary_tokens),
        })
    }

    /// 保存归档消息到记忆
    fn save_archived_messages(&mut self, agent_id: &AgentId, messages: &[Message]) -> Result<(), Error> {
        if let Some(memory) = &self.memory {
            let content = messages.iter()
                .map(|m| format!("{:?}: {}", m.role, m.content))
                .collect::<Vec<_>>()
                .join("\n");

            if !content.is_empty() {
                memory.store(
                    agent_id,
                    EventType::Other,
                    &format!("[ARCHIVED CONTEXT]\n{}", content),
                )?;
            }
        }
        Ok(())
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 上下文快照（用于持久化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSnapshot {
    pub messages: Vec<Message>,
    pub current_tokens: usize,
    pub compaction_policy: CompactionPolicy,
}

impl From<&Context> for ContextSnapshot {
    fn from(context: &Context) -> Self {
        Self {
            messages: context.messages.clone(),
            current_tokens: context.current_tokens,
            compaction_policy: context.compaction_policy,
        }
    }
}

impl From<ContextSnapshot> for Context {
    fn from(snapshot: ContextSnapshot) -> Self {
        Self {
            messages: snapshot.messages,
            max_tokens: 128_000,
            current_tokens: snapshot.current_tokens,
            compaction_policy: snapshot.compaction_policy,
        }
    }
}
