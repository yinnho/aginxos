use crate::core::{Agent, AgentId, AgentState, Goal, GoalId};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Agent 状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub id: AgentId,
    pub name: String,
    pub state: AgentState,
    pub current_goal: Option<GoalId>,
}

/// 系统状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemState {
    pub version: String,
    pub agents: Vec<AgentSnapshot>,
    pub pending_goals: Vec<Goal>,
    pub completed_goals: Vec<Goal>,
}

impl Default for SystemState {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            agents: vec![],
            pending_goals: vec![],
            completed_goals: vec![],
        }
    }
}

/// State Manager - 状态持久化
pub struct StateManager {
    state: SystemState,
    path: String,
}

impl StateManager {
    pub fn new(path: &str) -> Self {
        let state = Self::load(path).unwrap_or_default();
        Self {
            state,
            path: path.to_string(),
        }
    }

    /// 从文件加载状态
    fn load(path: &str) -> Result<SystemState, serde_json::Error> {
        if !Path::new(path).exists() {
            return Ok(SystemState::default());
        }
        let content = std::fs::read_to_string(path).unwrap_or_default();
        serde_json::from_str(&content)
    }

    /// 保存状态到文件
    pub fn save(&self) -> Result<(), crate::core::Error> {
        let content = serde_json::to_string_pretty(&self.state).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to serialize state: {}", e))
        })?;
        std::fs::write(&self.path, content).map_err(|e| {
            crate::core::Error::Internal(format!("Failed to write state file: {}", e))
        })
    }

    /// 注册 Agent
    pub fn register_agent(&mut self, agent: &Agent) {
        let snapshot = AgentSnapshot {
            id: agent.id.clone(),
            name: agent.name.clone(),
            state: agent.state,
            current_goal: agent.goal.as_ref().map(|g| g.id),
        };

        // 更新或添加
        if let Some(existing) = self.state.agents.iter_mut().find(|a| a.id == agent.id) {
            *existing = snapshot;
        } else {
            self.state.agents.push(snapshot);
        }
    }

    /// 添加待处理 Goal
    pub fn add_pending_goal(&mut self, goal: Goal) {
        self.state.pending_goals.push(goal);
    }

    /// 移动 Goal 到已完成
    pub fn complete_goal(&mut self, goal_id: GoalId) {
        if let Some(idx) = self.state.pending_goals.iter().position(|g| g.id == goal_id) {
            let goal = self.state.pending_goals.remove(idx);
            self.state.completed_goals.push(goal);
        }
    }

    /// 获取系统状态
    pub fn state(&self) -> &SystemState {
        &self.state
    }
}
