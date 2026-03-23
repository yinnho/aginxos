use serde::{Deserialize, Serialize};

/// 消息角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub name: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            name: None,
        }
    }

    pub fn tool_result(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            name: Some(name.into()),
        }
    }
}

/// 压缩策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionPolicy {
    /// 不压缩
    None,
    /// 保留最近 N 条消息
    KeepRecent(usize),
    /// 保留系统消息 + 最近 N 条
    SystemAndRecent(usize),
    /// 使用 LLM 摘要
    Summarize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self::SystemAndRecent(10)
    }
}

/// Context - 上下文窗口，是一级资源
#[derive(Debug, Clone)]
pub struct Context {
    /// 消息历史
    pub messages: Vec<Message>,
    /// 最大 token 数
    pub max_tokens: usize,
    /// 当前 token 使用量（估算）
    pub current_tokens: usize,
    /// 压缩策略
    pub compaction_policy: CompactionPolicy,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            messages: vec![],
            max_tokens: 128_000, // 默认 128k
            current_tokens: 0,
            compaction_policy: CompactionPolicy::default(),
        }
    }
}

impl Context {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            ..Default::default()
        }
    }

    /// 添加消息
    pub fn push(&mut self, message: Message) {
        let tokens = self.estimate_tokens(&message.content);
        self.messages.push(message);
        self.current_tokens += tokens;
    }

    /// 检查是否需要压缩
    pub fn needs_compaction(&self) -> bool {
        self.current_tokens > (self.max_tokens as f64 * 0.8) as usize
    }

    /// 估算 token 数量（简单实现：4 字符 ≈ 1 token）
    pub fn estimate_tokens(&self, text: &str) -> usize {
        text.len() / 4 + 1
    }

    /// 转换为 OpenAI 格式的消息
    pub fn to_openai_messages(&self) -> Vec<async_openai::types::ChatCompletionRequestMessage> {
        use async_openai::types::{
            ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessageArgs,
            ChatCompletionRequestUserMessageArgs,
            ChatCompletionRequestAssistantMessageArgs,
            ChatCompletionRequestToolMessageArgs,
        };

        self.messages.iter().map(|m| match m.role {
            super::Role::System => {
                ChatCompletionRequestMessage::System(
                    ChatCompletionRequestSystemMessageArgs::default()
                        .content(&*m.content)
                        .build()
                        .unwrap()
                )
            }
            super::Role::User => {
                ChatCompletionRequestMessage::User(
                    ChatCompletionRequestUserMessageArgs::default()
                        .content(&*m.content)
                        .build()
                        .unwrap()
                )
            }
            super::Role::Assistant => {
                ChatCompletionRequestMessage::Assistant(
                    ChatCompletionRequestAssistantMessageArgs::default()
                        .content(&*m.content)
                        .build()
                        .unwrap()
                )
            }
            super::Role::Tool => {
                ChatCompletionRequestMessage::Tool(
                    ChatCompletionRequestToolMessageArgs::default()
                        .content(&*m.content)
                        .tool_call_id(m.name.as_deref().unwrap_or("unknown"))
                        .build()
                        .unwrap()
                )
            }
        }).collect()
    }
}
