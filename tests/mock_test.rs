use agentos::core::*;
use agentos::harness::{ToolBus, ToolCall, ToolResult};

/// Mock LLM Gateway - 模拟 LLM 响应
struct MockLlmGateway {
    tool_bus: ToolBus,
    responses: Vec<MockResponse>,
    call_count: usize,
}

#[derive(Clone)]
struct MockResponse {
    content: String,
    tool_calls: Vec<ToolCall>,
}

impl MockLlmGateway {
    fn new(responses: Vec<MockResponse>) -> Self {
        Self {
            tool_bus: ToolBus::new(),
            responses,
            call_count: 0,
        }
    }

    fn chat(&mut self, _context: &Context) -> Result<MockLlmResponse, Error> {
        if self.call_count >= self.responses.len() {
            // 默认返回完成
            return Ok(MockLlmResponse {
                content: "Task completed successfully.".to_string(),
                tool_calls: vec![],
            });
        }

        let response = self.responses[self.call_count].clone();
        self.call_count += 1;

        Ok(MockLlmResponse {
            content: response.content,
            tool_calls: response.tool_calls,
        })
    }

    fn execute_tool(&self, agent: &Agent, call: &ToolCall) -> Result<ToolResult, Error> {
        self.tool_bus.execute(agent, call)
    }
}

struct MockLlmResponse {
    content: String,
    tool_calls: Vec<ToolCall>,
}

impl MockLlmResponse {
    fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// 模拟 Agent 执行器
struct MockAgentExecutor {
    agent: Agent,
    gateway: MockLlmGateway,
    max_iterations: usize,
}

impl MockAgentExecutor {
    fn new(agent: Agent, gateway: MockLlmGateway) -> Self {
        Self {
            agent,
            gateway,
            max_iterations: 10,
        }
    }

    async fn run(&mut self, goal: Goal) -> Result<StepResult, Error> {
        self.agent.assign_goal(goal.clone());

        self.agent.context.push(Message::system(
            "You are an agent that helps users accomplish tasks."
        ));
        self.agent.context.push(Message::user(&goal.description));

        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > self.max_iterations {
                return Ok(StepResult::Failed(Error::Internal(
                    "Max iterations exceeded".into()
                )));
            }

            let response = self.gateway.chat(&self.agent.context)?;

            self.agent.context.push(Message::assistant(&response.content));

            if !response.has_tool_calls() {
                self.agent.state = AgentState::Completed;
                return Ok(StepResult::Completed);
            }

            for tool_call in &response.tool_calls {
                let result = self.gateway.execute_tool(&self.agent, tool_call)?;

                self.agent.context.push(Message::tool_result(
                    &tool_call.name,
                    format!("Success: {}\n{}", result.success, result.output),
                ));
            }
        }
    }
}

// ===== 测试用例 =====

#[tokio::test]
async fn test_mock_simple_goal() {
    let agent = Agent::new("test_agent");
    let gateway = MockLlmGateway::new(vec![
        MockResponse {
            content: "I'll help you with that.".to_string(),
            tool_calls: vec![],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Say hello");

    let result = executor.run(goal).await;
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), StepResult::Completed));
}

#[tokio::test]
async fn test_mock_write_file() {
    let mut agent = Agent::new("file_writer");
    agent.grant_capability(Capability::FileSystem {
        paths: vec!["/tmp".into()],
        mode: AccessMode::ReadWrite,
    });

    let gateway = MockLlmGateway::new(vec![
        MockResponse {
            content: "I'll write a hello world file.".to_string(),
            tool_calls: vec![ToolCall {
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "/tmp/hello.txt",
                    "content": "Hello, World!"
                }),
            }],
        },
        MockResponse {
            content: "File created successfully.".to_string(),
            tool_calls: vec![],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Create a hello world file");

    let result = executor.run(goal).await;
    assert!(result.is_ok());

    // 验证文件是否创建
    assert!(std::path::Path::new("/tmp/hello.txt").exists());
    let content = std::fs::read_to_string("/tmp/hello.txt").unwrap();
    assert_eq!(content, "Hello, World!");

    // 清理
    std::fs::remove_file("/tmp/hello.txt").ok();
}

#[tokio::test]
async fn test_mock_read_file() {
    // 先创建测试文件
    std::fs::write("/tmp/test_read.txt", "Test content for reading").unwrap();

    let mut agent = Agent::new("file_reader");
    agent.grant_capability(Capability::FileSystem {
        paths: vec!["/tmp".into()],
        mode: AccessMode::ReadOnly,
    });

    let gateway = MockLlmGateway::new(vec![
        MockResponse {
            content: "I'll read the file.".to_string(),
            tool_calls: vec![ToolCall {
                name: "read_file".to_string(),
                arguments: serde_json::json!({
                    "path": "/tmp/test_read.txt"
                }),
            }],
        },
        MockResponse {
            content: "The file contains: Test content for reading".to_string(),
            tool_calls: vec![],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Read the test file");

    let result = executor.run(goal).await;
    assert!(result.is_ok());

    // 清理
    std::fs::remove_file("/tmp/test_read.txt").ok();
}

#[tokio::test]
async fn test_mock_capability_denied() {
    let agent = Agent::new("restricted_agent");
    // 不授予任何文件系统权限

    let gateway = MockLlmGateway::new(vec![
        MockResponse {
            content: "I'll try to read a file.".to_string(),
            tool_calls: vec![ToolCall {
                name: "read_file".to_string(),
                arguments: serde_json::json!({
                    "path": "/etc/passwd"
                }),
            }],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Read /etc/passwd");

    let result = executor.run(goal).await;
    // 应该因为权限不足而失败
    assert!(result.is_err() || matches!(result.unwrap(), StepResult::Failed(_)));
}

#[tokio::test]
async fn test_mock_execute_command() {
    let mut agent = Agent::new("command_runner");
    agent.grant_capability(Capability::Execute {
        commands: vec!["echo".into()],
    });

    let gateway = MockLlmGateway::new(vec![
        MockResponse {
            content: "I'll run echo command.".to_string(),
            tool_calls: vec![ToolCall {
                name: "execute".to_string(),
                arguments: serde_json::json!({
                    "command": "echo",
                    "args": ["Hello from AgentOS!"]
                }),
            }],
        },
        MockResponse {
            content: "Command executed successfully.".to_string(),
            tool_calls: vec![],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Run echo command");

    let result = executor.run(goal).await;
    assert!(result.is_ok());
    assert!(matches!(result.unwrap(), StepResult::Completed));
}

#[tokio::test]
async fn test_mock_multi_step_goal() {
    let mut agent = Agent::new("multi_step_agent");
    agent.grant_capability(Capability::FileSystem {
        paths: vec!["/tmp".into()],
        mode: AccessMode::ReadWrite,
    });
    agent.grant_capability(Capability::Execute {
        commands: vec!["cat".into()],
    });

    let gateway = MockLlmGateway::new(vec![
        // Step 1: Write file
        MockResponse {
            content: "Step 1: Creating file.".to_string(),
            tool_calls: vec![ToolCall {
                name: "write_file".to_string(),
                arguments: serde_json::json!({
                    "path": "/tmp/multi_test.txt",
                    "content": "Multi-step test content"
                }),
            }],
        },
        // Step 2: Read file to verify
        MockResponse {
            content: "Step 2: Verifying file.".to_string(),
            tool_calls: vec![ToolCall {
                name: "read_file".to_string(),
                arguments: serde_json::json!({
                    "path": "/tmp/multi_test.txt"
                }),
            }],
        },
        // Step 3: Done
        MockResponse {
            content: "All steps completed. File created and verified.".to_string(),
            tool_calls: vec![],
        },
    ]);

    let mut executor = MockAgentExecutor::new(agent, gateway);
    let goal = Goal::new("Create and verify a file");

    let result = executor.run(goal).await;
    assert!(result.is_ok());

    // 验证文件存在
    assert!(std::path::Path::new("/tmp/multi_test.txt").exists());

    // 清理
    std::fs::remove_file("/tmp/multi_test.txt").ok();
}

#[tokio::test]
async fn test_mock_priority_scheduling() {
    use agentos::scheduler::IntentScheduler;

    let mut scheduler = IntentScheduler::new();

    let low = Goal::new("Low priority task").with_priority(Priority::Low);
    let high = Goal::new("High priority task").with_priority(Priority::High);
    let critical = Goal::new("Critical task").with_priority(Priority::Critical);

    scheduler.enqueue(low);
    scheduler.enqueue(high);
    scheduler.enqueue(critical);

    // 应该按优先级顺序出队
    let first = scheduler.pop().unwrap();
    assert_eq!(first.priority, Priority::Critical);
    assert_eq!(first.description, "Critical task");

    let second = scheduler.pop().unwrap();
    assert_eq!(second.priority, Priority::High);

    let third = scheduler.pop().unwrap();
    assert_eq!(third.priority, Priority::Low);
}
