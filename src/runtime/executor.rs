use crate::core::*;
use crate::harness::MemoryManager;
use crate::llm::LlmGateway;
use crate::scheduler::IntentScheduler;

/// Agent 执行器
pub struct AgentExecutor {
    agent: Agent,
    gateway: LlmGateway,
    memory: MemoryManager,
    scheduler: IntentScheduler,
    max_iterations: usize,
}

impl AgentExecutor {
    pub fn new(
        agent: Agent,
        gateway: LlmGateway,
        memory: MemoryManager,
        scheduler: IntentScheduler,
    ) -> Self {
        Self {
            agent,
            gateway,
            memory,
            scheduler,
            max_iterations: 50,
        }
    }

    /// 执行 Agent 的主循环
    pub async fn run(&mut self, goal: Goal) -> Result<StepResult, Error> {
        self.agent.assign_goal(goal.clone());

        // 记录 Goal
        self.memory.store(
            &self.agent.id,
            crate::harness::EventType::UserInput,
            &format!("Goal: {}", goal.description),
        )?;

        // 初始化上下文
        self.agent.context.push(Message::system(
            "You are an agent that helps users accomplish tasks. \
             Use the available tools to complete the user's goal. \
             When you have completed the task, respond with a summary of what was done."
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

            // 调用 LLM
            let response = self.gateway.chat(&self.agent.context).await?;

            // 添加助手回复到上下文
            self.agent.context.push(Message::assistant(&response.content));

            if !response.has_tool_calls() {
                // 没有工具调用，认为任务完成
                self.memory.store(
                    &self.agent.id,
                    crate::harness::EventType::Completion,
                    &response.content,
                )?;

                self.agent.state = AgentState::Completed;
                return Ok(StepResult::Completed);
            }

            // 执行工具调用
            for tool_call in &response.tool_calls {
                self.memory.store(
                    &self.agent.id,
                    crate::harness::EventType::ToolCall,
                    &format!("{}: {:?}", tool_call.name, tool_call.arguments),
                )?;

                let result = self.gateway.execute_tool(&self.agent, tool_call)?;

                self.memory.store(
                    &self.agent.id,
                    crate::harness::EventType::ToolResult,
                    &result.output,
                )?;

                // 添加工具结果到上下文
                self.agent.context.push(Message::tool_result(
                    &tool_call.name,
                    &format!("Success: {}\n{}", result.success, result.output),
                ));
            }

            // 检查是否需要压缩上下文
            if self.agent.context.needs_compaction() {
                // MVP: 简单地截断旧消息（保留系统消息）
                // TODO: 实现 LLM 摘要
                self.compact_context();
            }
        }
    }

    /// 简单的上下文压缩
    fn compact_context(&mut self) {
        if self.agent.context.messages.len() <= 4 {
            return;
        }

        // 保留系统消息和最近的消息
        let system_messages: Vec<_> = self.agent.context.messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();

        let recent_count = 6;
        let recent_messages: Vec<_> = self.agent.context.messages
            .iter()
            .rev()
            .take(recent_count)
            .rev()
            .cloned()
            .collect();

        self.agent.context.messages = system_messages;
        self.agent.context.messages.extend(recent_messages);

        // 重新估算 token
        self.agent.context.current_tokens = self.agent.context.messages
            .iter()
            .map(|m| self.agent.context.estimate_tokens(&m.content))
            .sum();
    }
}
