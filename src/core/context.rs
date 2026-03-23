use serde::{Deserialize, Serialize};
use super::Error;

/// 消息角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// 工具调用信息（用于存储在消息中）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    pub name: Option<String>,
    /// 工具调用 ID（仅用于 Tool 角色）
    pub tool_call_id: Option<String>,
    /// 工具调用列表（仅用于 Assistant 角色）
    pub tool_calls: Vec<StoredToolCall>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls: vec![],
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<StoredToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            name: None,
            tool_call_id: None,
            tool_calls,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            name: None,
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: vec![],
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
    pub fn to_openai_messages(&self) -> Result<Vec<async_openai::types::ChatCompletionRequestMessage>, Error> {
        use async_openai::types::{
            ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessageArgs,
            ChatCompletionRequestUserMessageArgs,
            ChatCompletionRequestAssistantMessageArgs,
            ChatCompletionRequestToolMessageArgs,
            ChatCompletionMessageToolCall,
            FunctionCall,
        };

        self.messages.iter().map(|m| match m.role {
            super::Role::System => {
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(&*m.content)
                    .build()
                    .map(ChatCompletionRequestMessage::System)
            }
            super::Role::User => {
                ChatCompletionRequestUserMessageArgs::default()
                    .content(&*m.content)
                    .build()
                    .map(ChatCompletionRequestMessage::User)
            }
            super::Role::Assistant => {
                // 构建 Assistant 消息
                let openai_tool_calls: Vec<ChatCompletionMessageToolCall> = m.tool_calls.iter().map(|tc| {
                    ChatCompletionMessageToolCall {
                        id: tc.id.clone(),
                        r#type: async_openai::types::ChatCompletionToolType::Function,
                        function: FunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                    }
                }).collect();

                let content = if m.content.is_empty() {
                    None
                } else {
                    Some(&*m.content)
                };

                let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
                if let Some(c) = content {
                    builder.content(c);
                }
                if !openai_tool_calls.is_empty() {
                    builder.tool_calls(openai_tool_calls);
                }
                builder.build().map(ChatCompletionRequestMessage::Assistant)
            }
            super::Role::Tool => {
                let tool_id = m.tool_call_id.as_deref().unwrap_or("unknown");
                ChatCompletionRequestToolMessageArgs::default()
                    .content(&*m.content)
                    .tool_call_id(tool_id)
                    .build()
                    .map(ChatCompletionRequestMessage::Tool)
            }
        }).collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Internal(format!("Failed to build message: {}", e)))
    }
}
