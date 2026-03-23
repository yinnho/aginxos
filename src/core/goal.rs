use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Goal ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(Uuid);

impl GoalId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[derive(Default)]
pub enum Priority {
    Low = 0,
    #[default]
    Normal = 1,
    High = 2,
    Critical = 3,
}


/// 验证测试
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationTest {
    pub description: String,
    pub test_type: TestType,
    pub passes: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestType {
    /// 输出包含某字符串
    OutputContains(String),
    /// 文件存在
    FileExists(String),
    /// 命令执行成功
    CommandSucceeds(String),
    /// 自定义验证脚本
    Custom(String),
}

/// Goal - 意图/目标，是调度的基本单位
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: GoalId,
    pub description: String,
    pub success_criteria: Vec<VerificationTest>,
    pub priority: Priority,
    pub created_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
    pub dependencies: Vec<GoalId>,
}

impl Goal {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            id: GoalId::new(),
            description: description.into(),
            success_criteria: vec![],
            priority: Priority::default(),
            created_at: Utc::now(),
            deadline: None,
            dependencies: vec![],
        }
    }

    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_deadline(mut self, deadline: DateTime<Utc>) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn depends_on(mut self, goal_id: GoalId) -> Self {
        self.dependencies.push(goal_id);
        self
    }

    pub fn add_test(mut self, test: VerificationTest) -> Self {
        self.success_criteria.push(test);
        self
    }
}
