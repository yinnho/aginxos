use crate::core::*;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::path::Path;

/// 工具调用请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// 工具调用结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
}

/// 工具定义
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value, // JSON Schema
}

/// Tool Bus - 工具总线
pub struct ToolBus {
    tools: Vec<Tool>,
}

impl ToolBus {
    pub fn new() -> Self {
        Self {
            tools: vec![
                Tool {
                    name: "read_file".into(),
                    description: "Read the contents of a file".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "The path to the file to read"
                            }
                        },
                        "required": ["path"]
                    }),
                },
                Tool {
                    name: "write_file".into(),
                    description: "Write content to a file".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "The path to the file to write"
                            },
                            "content": {
                                "type": "string",
                                "description": "The content to write to the file"
                            }
                        },
                        "required": ["path", "content"]
                    }),
                },
                Tool {
                    name: "execute".into(),
                    description: "Execute a shell command".into(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "command": {
                                "type": "string",
                                "description": "The command to execute"
                            },
                            "args": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Command arguments"
                            }
                        },
                        "required": ["command"]
                    }),
                },
            ],
        }
    }

    /// 获取所有工具定义（OpenAI 格式）
    pub fn to_openai_tools(&self) -> Vec<async_openai::types::ChatCompletionTool> {
        use async_openai::types::*;

        self.tools.iter().map(|tool| {
            ChatCompletionTool {
                r#type: ChatCompletionToolType::Function,
                function: FunctionObject {
                    name: tool.name.clone(),
                    description: Some(tool.description.clone()),
                    parameters: Some(tool.parameters.clone()),
                    strict: Some(true),
                },
            }
        }).collect()
    }

    /// 执行工具调用
    pub fn execute(&self, agent: &Agent, call: &ToolCall) -> Result<ToolResult, Error> {
        // 检查权限
        self.check_capability(agent, call)?;

        // 执行工具
        let result = match call.name.as_str() {
            "read_file" => self.read_file(&call.arguments)?,
            "write_file" => self.write_file(&call.arguments)?,
            "execute" => self.execute_command(&call.arguments)?,
            _ => return Err(Error::Tool(format!("Unknown tool: {}", call.name))),
        };

        Ok(result)
    }

    /// 检查 Agent 是否有执行该工具的能力
    fn check_capability(&self, agent: &Agent, call: &ToolCall) -> Result<(), Error> {
        match call.name.as_str() {
            "read_file" | "write_file" => {
                let path = call.arguments["path"].as_str().unwrap_or("");
                let mode = if call.name == "read_file" {
                    AccessMode::ReadOnly
                } else {
                    AccessMode::WriteOnly
                };
                let cap = Capability::FileSystem {
                    paths: vec![path.into()],
                    mode,
                };
                if !agent.has_capability(&cap) {
                    return Err(Error::CapabilityDenied(format!("No access to path: {}", path)));
                }
            }
            "execute" => {
                let cmd = call.arguments["command"].as_str().unwrap_or("");
                let cap = Capability::Execute {
                    commands: vec![cmd.into()],
                };
                if !agent.has_capability(&cap) {
                    return Err(Error::CapabilityDenied(format!("Cannot execute: {}", cmd)));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn read_file(&self, args: &serde_json::Value) -> Result<ToolResult, Error> {
        let path = args["path"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'path' argument".into())
        })?;

        match std::fs::read_to_string(path) {
            Ok(content) => Ok(ToolResult {
                success: true,
                output: content,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Error reading file: {}", e),
            }),
        }
    }

    fn write_file(&self, args: &serde_json::Value) -> Result<ToolResult, Error> {
        let path = args["path"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'path' argument".into())
        })?;
        let content = args["content"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'content' argument".into())
        })?;

        // 确保父目录存在
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Tool(format!("Failed to create directory: {}", e))
            })?;
        }

        match std::fs::write(path, content) {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Successfully wrote to {}", path),
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Error writing file: {}", e),
            }),
        }
    }

    fn execute_command(&self, args: &serde_json::Value) -> Result<ToolResult, Error> {
        let command = args["command"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'command' argument".into())
        })?;
        let args_vec: Vec<&str> = args["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let output = Command::new(command)
            .args(&args_vec)
            .output()
            .map_err(|e| Error::Tool(format!("Failed to execute command: {}", e)))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let result = if output.status.success() {
            ToolResult {
                success: true,
                output: stdout.into_owned(),
            }
        } else {
            ToolResult {
                success: false,
                output: format!("{}\n{}", stdout, stderr),
            }
        };

        Ok(result)
    }
}

impl Default for ToolBus {
    fn default() -> Self {
        Self::new()
    }
}
