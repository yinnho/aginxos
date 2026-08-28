//! Simple task management for spawned background processes

use std::sync::atomic::{AtomicUsize, Ordering};

static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);

struct TaskEntry {
    pid: u32,
    cmd: &'static str,
}

// Simple fixed-size task table (lock-free, single writer)
const MAX_TASKS: usize = 64;
static mut TASKS: [Option<(u32, &'static str)>; MAX_TASKS] = [None; MAX_TASKS];

/// Register a spawned process
pub fn register(pid: u32, cmd: &str) {
    let idx = TASK_COUNT.fetch_add(1, Ordering::Relaxed) % MAX_TASKS;
    // Leak the string to get a 'static reference
    let static_cmd: &'static str = Box::leak(cmd.to_string().into_boxed_str());
    unsafe {
        TASKS[idx] = Some((pid, static_cmd));
    }
}

/// List all registered tasks
pub fn list() -> Vec<(u32, String)> {
    let mut result = Vec::new();
    unsafe {
        for i in 0..MAX_TASKS {
            if let Some((pid, cmd)) = &TASKS[i] {
                // Check if process is still alive
                let alive = unsafe { libc::kill(*pid as i32, 0) == 0 };
                if alive {
                    result.push((*pid, cmd.to_string()));
                }
            }
        }
    }
    result
}
