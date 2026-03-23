use crate::core::*;
use crate::harness::{ContextManager, MemoryManager};
use crate::llm::LlmGateway;
use crate::scheduler::IntentScheduler;

/// Agent 执行器
#[allow(dead_code)] // scheduler reserved for multi-agent coordination
pub struct AgentExecutor {
    agent: Agent,
    gateway: LlmGateway,
    memory: MemoryManager,
    #[allow(dead_code)] // context_manager reserved for advanced compaction
    context_manager: ContextManager,
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
        // ContextManager 用于上下文压缩，memory 用于事件记录
        // 由于 MemoryManager 不可 Clone，这里只保留引用的语义
        // 压缩时归档的消息会保存到主 memory 中
        Self {
            agent,
            gateway,
            memory,
            context_manager: ContextManager::new(),
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
                    format!("Success: {}\n{}", result.success, result.output),
                ));
            }

            // 检查并执行上下文压缩
            // 使用 ContextManager 的压缩策略（默认保留系统消息 + 最近 10 条）
            self.context_manager.maybe_compact(&mut self.agent).await?;
        }
    }
}
