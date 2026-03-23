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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Placeholder API key for services that don't require authentication
/// This is a well-known placeholder that clearly indicates no real key is needed
const NO_AUTH_PLACEHOLDER: &str = "no-api-key-required";

/// 速率限制配置
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 每分钟最大请求数
    pub max_requests_per_minute: u32,
    /// 最大 tokens 每分钟
    pub max_tokens_per_minute: u64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests_per_minute: 60,
            max_tokens_per_minute: 100_000,
        }
    }
}

/// 使用统计
#[derive(Debug, Default)]
pub struct UsageStats {
    /// API 调用次数
    pub total_requests: u64,
    /// 总输入 tokens
    pub total_input_tokens: u64,
    /// 总输出 tokens
    pub total_output_tokens: u64,
    /// 工具调用次数
    pub total_tool_calls: u64,
}

/// LLM Gateway - LLM 网关
pub struct LlmGateway {
    client: Client<OpenAIConfig>,
    model: String,
    tool_bus: ToolBus,
    provider: LlmProvider,
    rate_limit: RateLimitConfig,
    // 使用原子计数器实现线程安全的统计
    request_count: AtomicU64,
    token_count: AtomicU64,
    last_reset: std::sync::Mutex<Instant>,
}

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
            rate_limit: RateLimitConfig::default(),
            request_count: AtomicU64::new(0),
            token_count: AtomicU64::new(0),
            last_reset: std::sync::Mutex::new(Instant::now()),
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
            // Ollama 本地模型通常不需要速率限制
            rate_limit: RateLimitConfig {
                max_requests_per_minute: 1000,
                max_tokens_per_minute: 1_000_000,
            },
            request_count: AtomicU64::new(0),
            token_count: AtomicU64::new(0),
            last_reset: std::sync::Mutex::new(Instant::now()),
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
            rate_limit: RateLimitConfig::default(),
            request_count: AtomicU64::new(0),
            token_count: AtomicU64::new(0),
            last_reset: std::sync::Mutex::new(Instant::now()),
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

    /// 检查并等待速率限制
    fn check_rate_limit(&self) -> Result<(), Error> {
        let mut last_reset = self.last_reset.lock()
            .map_err(|_| Error::Internal("Failed to acquire lock".into()))?;

        let elapsed = last_reset.elapsed();

        // 每分钟重置计数器
        if elapsed >= Duration::from_secs(60) {
            self.request_count.store(0, Ordering::SeqCst);
            self.token_count.store(0, Ordering::SeqCst);
            *last_reset = Instant::now();
        }

        // 检查请求限制
        let current_requests = self.request_count.load(Ordering::SeqCst);
        if current_requests >= self.rate_limit.max_requests_per_minute as u64 {
            return Err(Error::Llm(
                format!("Rate limit exceeded: {} requests/minute", self.rate_limit.max_requests_per_minute)
            ));
        }

        Ok(())
    }

    /// 获取当前使用统计
    pub fn usage_stats(&self) -> UsageStats {
        UsageStats {
            total_requests: self.request_count.load(Ordering::SeqCst),
            total_input_tokens: 0, // 需要从响应中获取
            total_output_tokens: self.token_count.load(Ordering::SeqCst),
            total_tool_calls: 0,
        }
    }

    /// 发送消息并获取响应
    pub async fn chat(
        &self,
        context: &Context,
    ) -> Result<LlmResponse, Error> {
        // 检查速率限制
        self.check_rate_limit()?;

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

        // 更新请求计数
        self.request_count.fetch_add(1, Ordering::SeqCst);

        let choice = response.choices.first()
            .ok_or_else(|| Error::Llm("No response choices".into()))?;

        let message = &choice.message;

        // 记录 token 使用情况（如果 API 返回）
        if let Some(usage) = response.usage.as_ref() {
            self.token_count.fetch_add(usage.total_tokens as u64, Ordering::SeqCst);
        }

        // 检查是否有工具调用
        let tool_calls = message.tool_calls.clone().unwrap_or_default();

        Ok(LlmResponse {
            content: message.content.clone().unwrap_or_default(),
            tool_calls: tool_calls.into_iter().map(|tc| ToolCall {
                name: tc.function.name,
                arguments: serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null),
            }).collect(),
            usage: response.usage.map(|u| TokenUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            }),
        })
    }

    /// 执行工具调用
    pub fn execute_tool(&self, agent: &Agent, call: &ToolCall) -> Result<ToolResult, Error> {
        self.tool_bus.execute(agent, call)
    }
}

/// Token 使用情况
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// LLM 响应
#[derive(Debug)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
}

impl LlmResponse {
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}
