use crate::core::*;
use crate::harness::{ToolBus, ToolCall, ToolResult};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        CreateChatCompletionRequestArgs,
        ChatCompletionTool,
    },
};
use serde_json::Value;

/// LLM 提供商
#[derive(Debug, Clone)]
pub enum LlmProvider {
    OpenAI,
    Anthropic,
    Local(String), // 本地模型路径
}

/// LLM Gateway - LLM 网关
pub struct LlmGateway {
    client: Client<OpenAIConfig>,
    model: String,
    tool_bus: ToolBus,
}

impl LlmGateway {
    pub fn new(api_key: Option<String>, model: Option<String>) -> Self {
        let config = if let Some(key) = api_key {
            OpenAIConfig::new().with_api_key(key)
        } else {
            OpenAIConfig::new()
        };

        let client = Client::with_config(config);

        Self {
            client,
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
            tool_bus: ToolBus::new(),
        }
    }

    /// 获取工具定义
    pub fn tools(&self) -> Vec<ChatCompletionTool> {
        self.tool_bus.to_openai_tools()
    }

    /// 发送消息并获取响应
    pub async fn chat(
        &self,
        context: &Context,
    ) -> Result<LlmResponse, Error> {
        let messages = context.to_openai_messages();
        let tools = self.tools();

        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages(messages)
            .tools(tools)
            .tool_choice("auto")
            .build()
            .map_err(|e| Error::Llm(format!("Failed to build request: {}", e)))?;

        let response = self.client
            .chat()
            .create(request)
            .await
            .map_err(|e| Error::Llm(format!("API error: {}", e)))?;

        let choice = response.choices.first()
            .ok_or_else(|| Error::Llm("No response choices".into()))?;

        let message = &choice.message;

        // 检查是否有工具调用
        let tool_calls = message.tool_calls.clone().unwrap_or_default();

        Ok(LlmResponse {
            content: message.content.clone().unwrap_or_default(),
            tool_calls: tool_calls.into_iter().map(|tc| ToolCall {
                name: tc.function.name,
                arguments: serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null),
            }).collect(),
        })
    }

    /// 执行工具调用
    pub fn execute_tool(&self, agent: &Agent, call: &ToolCall) -> Result<ToolResult, Error> {
        self.tool_bus.execute(agent, call)
    }
}

/// LLM 响应
#[derive(Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

impl LlmResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}
