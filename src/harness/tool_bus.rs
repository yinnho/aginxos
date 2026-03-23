use crate::core::*;
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// 命令执行配置
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    /// 执行超时（秒）
    pub timeout_secs: u64,
    /// 禁止执行的命令黑名单
    pub blocked_commands: Vec<String>,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            // 危险命令黑名单
            blocked_commands: vec![
                "rm".into(), "rmdir".into(), "dd".into(),
                "mkfs".into(), "fdisk".into(), "chmod".into(),
                "chown".into(), "sudo".into(), "su".into(),
                "passwd".into(), "shutdown".into(), "reboot".into(),
                "init".into(), "kill".into(), "killall".into(),
                "curl".into(), "wget".into(),  // 禁止网络下载
            ],
        }
    }
}

/// Tool Bus - 工具总线
pub struct ToolBus {
    tools: Vec<Tool>,
    exec_config: ExecutionConfig,
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
            exec_config: ExecutionConfig::default(),
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
            "read_file" => self.read_file(agent, &call.arguments)?,
            "write_file" => self.write_file(agent, &call.arguments)?,
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

    /// 验证路径是否在允许的范围内（防止路径遍历攻击）
    fn validate_path(&self, path_str: &str, allowed_paths: &[PathBuf]) -> Result<PathBuf, Error> {
        let path = PathBuf::from(path_str);

        // 规范化路径（解析 ..、. 和符号链接）
        let canonical_path = if path.exists() {
            path.canonicalize()
                .map_err(|e| Error::Tool(format!("Failed to resolve path: {}", e)))?
        } else {
            // 文件不存在时，规范化父目录
            let parent = path.parent().unwrap_or(Path::new("."));
            let canonical_parent = parent.canonicalize()
                .map_err(|e| Error::Tool(format!("Failed to resolve parent path: {}", e)))?;
            canonical_parent.join(path.file_name().unwrap_or_default())
        };

        // 检查是否在允许的路径范围内
        let is_allowed = allowed_paths.iter().any(|allowed| {
            canonical_path.starts_with(allowed)
        });

        if !is_allowed {
            return Err(Error::CapabilityDenied(
                format!("Path '{}' is outside allowed directories", path_str)
            ));
        }

        Ok(canonical_path)
    }

    /// 从 Agent 能力中提取允许的文件系统路径
    fn get_allowed_paths(&self, agent: &Agent) -> Vec<PathBuf> {
        agent.capabilities.iter()
            .filter_map(|cap| {
                if let Capability::FileSystem { paths, .. } = cap {
                    // 尝试规范化允许的路径
                    paths.iter()
                        .filter_map(|p| p.canonicalize().ok())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .next()
                } else {
                    None
                }
            })
            .collect()
    }

    fn read_file(&self, agent: &Agent, args: &serde_json::Value) -> Result<ToolResult, Error> {
        let path_str = args["path"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'path' argument".into())
        })?;

        // 获取允许的路径并验证
        let allowed_paths = self.get_allowed_paths(agent);
        let safe_path = self.validate_path(path_str, &allowed_paths)?;

        match std::fs::read_to_string(&safe_path) {
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

    fn write_file(&self, agent: &Agent, args: &serde_json::Value) -> Result<ToolResult, Error> {
        let path_str = args["path"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'path' argument".into())
        })?;
        let content = args["content"].as_str().ok_or_else(|| {
            Error::Tool("Missing 'content' argument".into())
        })?;

        // 获取允许的路径并验证
        let allowed_paths = self.get_allowed_paths(agent);
        let safe_path = self.validate_path(path_str, &allowed_paths)?;

        // 确保父目录存在
        if let Some(parent) = safe_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Tool(format!("Failed to create directory: {}", e))
            })?;
        }

        match std::fs::write(&safe_path, content) {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Successfully wrote to {}", path_str),
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

        // 检查命令黑名单
        let cmd_name = Path::new(command)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(command);

        if self.exec_config.blocked_commands.iter().any(|blocked| {
            blocked == cmd_name || blocked == command
        }) {
            return Err(Error::CapabilityDenied(
                format!("Command '{}' is blocked for security reasons", command)
            ));
        }

        let args_vec: Vec<&str> = args["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // 使用 spawn 替代 output()，以便设置超时
        let mut cmd = Command::new(command);
        cmd.args(&args_vec);

        let child = cmd.spawn().map_err(|e| {
            Error::Tool(format!("Failed to spawn command: {}", e))
        })?;

        // 等待命令完成
        // TODO: 实现真正的超时机制需要使用 tokio::process
        // 目前使用 spawn + wait_with_output 的同步方式
        // 完整实现需要: tokio::time::timeout + child.kill()
        let _timeout = Duration::from_secs(self.exec_config.timeout_secs);
        let result = child.wait_with_output();

        let output = match result {
            Ok(o) => o,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Command execution failed: {}", e),
                });
            }
        };

        // 注意：这里简化了超时处理
        // 完整实现需要使用 tokio::process 或 kill 超时进程
        // 由于 std::process::Command 是同步的，这里只做记录

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
