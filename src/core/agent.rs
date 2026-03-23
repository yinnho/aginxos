use crate::core::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

/// Agent ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(Uuid);

impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AgentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(AgentId)
    }
}

/// Agent 状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    /// 空闲，等待分配 Goal
    Idle,
    /// 正在执行
    Running,
    /// 暂停（等待资源或用户输入）
    Paused,
    /// 阻塞（等待依赖）
    Blocked,
    /// 已完成
    Completed,
    /// 失败
    Failed,
}

/// Agent - 有目标的执行单元
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub state: AgentState,
    pub goal: Option<Goal>,
    pub context: Context,
    pub capabilities: Vec<Capability>,
}

impl Agent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: AgentId::new(),
            name: name.into(),
            state: AgentState::Idle,
            goal: None,
            context: Context::default(),
            capabilities: vec![
                // 默认基本能力
                Capability::FileSystem {
                    paths: vec![".".into()],
                    mode: AccessMode::ReadWrite,
                },
                Capability::Execute {
                    commands: vec!["ls".into(), "cat".into(), "echo".into()],
                },
            ],
        }
    }

    /// 分配 Goal 给 Agent
    pub fn assign_goal(&mut self, goal: Goal) {
        self.goal = Some(goal);
        self.state = AgentState::Running;
    }

    /// 检查是否有某个能力
    pub fn has_capability(&self, cap: &Capability) -> bool {
        self.capabilities.iter().any(|c| c.allows(cap))
    }

    /// 授予能力
    pub fn grant_capability(&mut self, cap: Capability) {
        if !self.has_capability(&cap) {
            self.capabilities.push(cap);
        }
    }

    /// 撤销能力
    pub fn revoke_capability(&mut self, cap: &Capability) {
        self.capabilities.retain(|c| !c.matches(cap));
    }
}

/// Agent 执行一步的结果
#[derive(Debug)]
pub enum StepResult {
    /// 有进展，继续执行
    Progress,
    /// 被阻塞，需要等待
    Blocked(String),
    /// Goal 完成
    Completed,
    /// 执行失败
    Failed(Error),
}

/// 错误类型
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("Capability denied: {0}")]
    CapabilityDenied(String),

    #[error("Context exceeded: {0}")]
    ContextExceeded(String),

    #[error("Internal error: {0}")]
    Internal(String),
}
