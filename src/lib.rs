//! AgentOS - An Agent Harness Operating System
//!
//! 核心理念：以"意图"为调度单位，以"上下文"为稀缺资源

pub mod core;
pub mod scheduler;
pub mod runtime;
pub mod harness;
pub mod llm;
pub mod cli;

pub use core::{Agent, Goal, Context, Capability};
