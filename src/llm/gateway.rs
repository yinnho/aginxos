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

/// Placeholder API key for services that don't require authentication
/// This is a well-known placeholder that clearly indicates no real key is needed
const NO_AUTH_PLACEHOLDER: &str = "no-api-key-required";

/// LLM 提供商
#[derive(Debug, Clone)]
pub enum LlmProvider {
    /// OpenAI API
    OpenAI,
    /// Ollama 本地服务
    Ollama { base_url: String },
    /// 自定义 OpenAI 兼容端点
    Custom { base_url: String, api_key: Option<String> },
}

impl Default for LlmProvider {
    fn default() -> Self {
        Self::OpenAI
    }
}

/// LLM Gateway - LLM 网关
pub struct LlmGateway {
    client: Client<OpenAIConfig>,
    model: String,
    tool_bus: ToolBus,
    provider: LlmProvider,
}

impl LlmGateway {
    /// 创建 OpenAI 网关
    pub fn openai(api_key: Option<String>, model: Option<String>) -> Self {
        let config = if let Some(key) = api_key {
            OpenAIConfig::new().with_api_key(key)
        } else {
            OpenAIConfig::new()
        };

        Self {
            client: Client::with_config(config),
            model: model.unwrap_or_else(|| "gpt-4o".to_string()),
            tool_bus: ToolBus::new(),
            provider: LlmProvider::OpenAI,
        }
    }

    /// 创建 Ollama 网关 (本地 LLM)
    pub fn ollama(base_url: Option<String>, model: Option<String>) -> Self {
        let url = base_url.unwrap_or_else(|| "http://localhost:11434/v1".to_string());
        // Ollama 不需要 API key，使用占位符
        let config = OpenAIConfig::new()
            .with_api_base(&url)
            .with_api_key(NO_AUTH_PLACEHOLDER);

        Self {
            client: Client::with_config(config),
            model: model.unwrap_or_else(|| "llama3.2".to_string()),
            tool_bus: ToolBus::new(),
            provider: LlmProvider::Ollama { base_url: url },
        }
    }

    /// 创建自定义端点网关
    pub fn custom(base_url: String, api_key: Option<String>, model: Option<String>) -> Self {
        let config = if let Some(key) = &api_key {
            OpenAIConfig::new()
                .with_api_base(&base_url)
                .with_api_key(key)
        } else {
            // 自定义端点可能不需要 key，使用占位符
            OpenAIConfig::new()
                .with_api_base(&base_url)
                .with_api_key(NO_AUTH_PLACEHOLDER)
        };

        Self {
            client: Client::with_config(config),
            model: model.unwrap_or_else(|| "local-model".to_string()),
            tool_bus: ToolBus::new(),
            provider: LlmProvider::Custom { base_url, api_key },
        }
    }

    /// 从环境变量自动创建网关
    pub fn from_env() -> Self {
        // 优先检查本地 LLM 配置
        if let Ok(provider) = std::env::var("AGENTOS_LLM_PROVIDER") {
            match provider.to_lowercase().as_str() {
                "ollama" | "local" => {
                    return Self::ollama(
                        std::env::var("OLLAMA_BASE_URL").ok(),
                        std::env::var("AGENTOS_MODEL").ok(),
                    );
                }
                "custom" => {
                    if let Ok(base_url) = std::env::var("AGENTOS_LLM_BASE_URL") {
                        return Self::custom(
                            base_url,
                            std::env::var("AGENTOS_LLM_API_KEY").ok(),
                            std::env::var("AGENTOS_MODEL").ok(),
                        );
                    }
                }
                _ => {}
            }
        }

        // 检查是否有 Ollama 运行
        if Self::check_ollama_running() {
            println!("ℹ Detected Ollama running, using local LLM");
            return Self::ollama(
                std::env::var("OLLAMA_BASE_URL").ok(),
                std::env::var("AGENTOS_MODEL").ok().or(std::env::var("OLLAMA_MODEL").ok()),
            );
        }

        // 默认使用 OpenAI
        Self::openai(
            std::env::var("OPENAI_API_KEY").ok(),
            std::env::var("AGENTOS_MODEL").ok(),
        )
    }

    /// 检查 Ollama 是否在运行
    fn check_ollama_running() -> bool {
        use std::net::TcpStream;

        let url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());

        // 解析主机和端口
        let host_port = url
            .trim_start_matches("http://")
            .trim_start_matches("https://");

        TcpStream::connect(host_port).is_ok()
    }

    /// 获取当前提供商
    pub fn provider(&self) -> &LlmProvider {
        &self.provider
    }

    /// 获取当前模型
    pub fn model(&self) -> &str {
        &self.model
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
        let messages = context.to_openai_messages()?;
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
