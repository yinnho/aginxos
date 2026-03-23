use crate::core::*;
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

/// 调度器中的 Goal 封装（支持优先级队列）
#[derive(Debug, Clone)]
struct ScheduledGoal {
    goal: Goal,
    /// 入队时间（用于 FIFO 相同优先级）
    enqueued_at: std::time::Instant,
}

impl PartialEq for ScheduledGoal {
    fn eq(&self, other: &Self) -> bool {
        self.goal.id == other.goal.id
    }
}

impl Eq for ScheduledGoal {}

impl PartialOrd for ScheduledGoal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledGoal {
    fn cmp(&self, other: &Self) -> Ordering {
        // 优先级高的先执行（反向，因为 BinaryHeap 是最大堆）
        // 相同优先级则先入先出
        self.goal.priority
            .cmp(&other.goal.priority)
            .then_with(|| other.enqueued_at.cmp(&self.enqueued_at))
    }
}

/// Intent Scheduler - 意图调度器
pub struct IntentScheduler {
    /// Goal 优先级队列
    goal_queue: BinaryHeap<ScheduledGoal>,
    /// 已完成的 Goal
    completed: HashMap<GoalId, Goal>,
    /// Goal 依赖关系
    dependencies: HashMap<GoalId, Vec<GoalId>>,
}

impl IntentScheduler {
    pub fn new() -> Self {
        Self {
            goal_queue: BinaryHeap::new(),
            completed: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    /// 添加 Goal 到队列
    pub fn enqueue(&mut self, goal: Goal) {
        // 记录依赖关系
        for dep_id in &goal.dependencies {
            self.dependencies
                .entry(*dep_id)
                .or_default()
                .push(goal.id);
        }

        self.goal_queue.push(ScheduledGoal {
            goal,
            enqueued_at: std::time::Instant::now(),
        });
    }

    /// 获取下一个可执行的 Goal（考虑依赖）
    pub fn next(&self) -> Option<&Goal> {
        self.goal_queue
            .iter()
            .find(|sg| self.dependencies_satisfied(&sg.goal))
            .map(|sg| &sg.goal)
    }

    /// 弹出下一个可执行的 Goal
    pub fn pop(&mut self) -> Option<Goal> {
        // 找到第一个依赖满足的 Goal
        let goal_id = self.goal_queue
            .iter()
            .find(|sg| self.dependencies_satisfied(&sg.goal))
            .map(|sg| sg.goal.id)?;

        // 从堆中移除（需要重建堆）
        let goal = self.goal_queue
            .iter()
            .find(|sg| sg.goal.id == goal_id)?
            .goal
            .clone();

        self.goal_queue = self.goal_queue
            .drain()
            .filter(|sg| sg.goal.id != goal_id)
            .collect();

        Some(goal)
    }

    /// 检查 Goal 的依赖是否都满足
    fn dependencies_satisfied(&self, goal: &Goal) -> bool {
        goal.dependencies.iter().all(|dep_id| self.completed.contains_key(dep_id))
    }

    /// 标记 Goal 完成
    pub fn complete(&mut self, goal: Goal) {
        self.completed.insert(goal.id, goal);
    }

    /// 队列中的 Goal 数量
    pub fn pending_count(&self) -> usize {
        self.goal_queue.len()
    }

    /// 已完成的 Goal 数量
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// 是否有等待执行的 Goal
    pub fn has_pending(&self) -> bool {
        self.goal_queue.iter().any(|sg| self.dependencies_satisfied(&sg.goal))
    }
}

impl Default for IntentScheduler {
    fn default() -> Self {
        Self::new()
    }
}
