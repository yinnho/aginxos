//! Task management - Phase 7 multitasking
//!
//! Provides preemptive multitasking for kernel threads via timer interrupts.

use alloc::alloc::{alloc_zeroed, Layout};
use core::sync::atomic::{AtomicU32, Ordering};

const MAILBOX_SIZE: usize = 256;
pub const MAX_FDS: usize = 16;
pub const FD_NAME_LEN: usize = 64;

/// File descriptor entry (per-task, like mailboxes)
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FdEntry {
    pub name: [u8; FD_NAME_LEN],
    pub offset: u64,
    pub flags: u32,
    pub used: bool,
}

// Per-task FD tables (separate from Task struct, indexed by task_idx * MAX_FDS + fd_idx)
static mut FD_TABLES: [[FdEntry; MAX_FDS]; MAX_TASKS] = [[FdEntry {
    name: [0; FD_NAME_LEN], offset: 0, flags: 0, used: false
}; MAX_FDS]; MAX_TASKS];

/// Fixed-size ring buffer for IPC mailbox (no heap allocation)
struct Mailbox {
    buf: [u8; MAILBOX_SIZE],
    head: usize,
    len: usize,
}

impl Mailbox {
    #[allow(dead_code)]
    const fn new() -> Self {
        Self { buf: [0; MAILBOX_SIZE], head: 0, len: 0 }
    }

    fn push_back(&mut self, b: u8) {
        if self.len < MAILBOX_SIZE {
            let idx = (self.head + self.len) % MAILBOX_SIZE;
            self.buf[idx] = b;
            self.len += 1;
        }
    }

    fn pop_front(&mut self) -> Option<u8> {
        if self.len == 0 {
            None
        } else {
            let b = self.buf[self.head];
            self.head = (self.head + 1) % MAILBOX_SIZE;
            self.len -= 1;
            Some(b)
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Task states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,     // Runnable, waiting for CPU
    Running,   // Currently executing
    #[allow(dead_code)]
    Blocked,   // Waiting for event
    Dead,      // Finished, slot reusable
}

/// Task Control Block
pub struct Task {
    pub id: u32,
    pub kernel_sp: u64,        // Saved SP when not running
    pub state: TaskState,
    pub name: &'static str,
    pub user_ttbr0: u64,       // User page table address (0 for kernel tasks)
    pub user_sp: u64,          // SP_EL0 value for user tasks
    pub is_user: bool,         // True if EL0 task
}

const MAX_TASKS: usize = 16;
const STACK_SIZE: usize = 65536; // 64KB per task stack (FS operations need large buffers)

// Separate mailbox storage to keep Task struct small
// (avoids BSS overlap with page_table section)
static mut MAILBOXES: [Mailbox; MAX_TASKS] = [
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
    Mailbox { buf: [0; MAILBOX_SIZE], head: 0, len: 0 },
];// Task table
pub static mut TASKS: [Option<Task>; MAX_TASKS] = [
    None, None, None, None, None, None, None, None,
    None, None, None, None, None, None, None, None,
];
pub static mut CURRENT: usize = 0;
static TASK_COUNT: AtomicU32 = AtomicU32::new(0);

// Globals accessed from assembly
#[no_mangle]
static mut OLD_TASK_SP_PTR: usize = 0;

#[no_mangle]
static mut NEW_TASK_SP: usize = 0;

use crate::platform::UART;

// Conditional UART imports
#[cfg(not(feature = "board-redfin"))]
use crate::uart;
#[cfg(feature = "board-redfin")]
use crate::qup_uart as uart;

/// Idle task - runs when no other tasks are ready
extern "C" fn idle_task() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi"); }
    }
}

/// Initialize task frame on stack for kernel task (EL1)
/// Returns the SP value that the IRQ restore code expects
unsafe fn init_task_stack(func: extern "C" fn() -> !) -> usize {
    // Allocate 8KB stack, 16-byte aligned
    let layout = Layout::from_size_align(STACK_SIZE, 16).unwrap();
    let stack_base = alloc_zeroed(layout) as usize;
    let stack_top = stack_base + STACK_SIZE;

    // Frame layout (272 bytes total):
    // SP+0 to SP+240: x0-x30 (31 regs x 8 bytes)
    // SP+256: ELR_EL1
    // SP+264: SPSR_EL1

    let frame_base = stack_top - 272;

    // Write ELR_EL1 at offset 256 (where task will start)
    core::ptr::write_volatile((frame_base + 256) as *mut u64, func as usize as u64);

    // Write SPSR_EL1 at offset 264 (EL1h, IRQ unmasked)
    // M[3:0] = 0b0101 = EL1h (using SP_EL1)
    // DAIF = 0 (all interrupts unmasked)
    core::ptr::write_volatile((frame_base + 264) as *mut u64, 0x5);

    frame_base
}

/// Initialize task frame for EL0 user task
/// kernel_sp: kernel stack for exception handling
/// user_entry: user code entry point
/// user_stack_top: top of user stack (SP_EL0)
unsafe fn init_user_task_stack(kernel_sp_base: usize, user_entry: usize, user_stack_top: usize) -> usize {
    let kernel_stack_top = kernel_sp_base + STACK_SIZE;

    // Set up kernel stack frame that ERETs to EL0
    let frame_base = kernel_stack_top - 272;

    // ELR_EL1 = user entry point
    core::ptr::write_volatile((frame_base + 256) as *mut u64, user_entry as u64);

    // SPSR_EL1 = EL0t (M=0b0000), all interrupts unmasked
    // When ERET executes, CPU drops to EL0
    core::ptr::write_volatile((frame_base + 264) as *mut u64, 0x0u64);

    // Set x0 (at offset 0) = user_stack_top so user code knows its stack
    // Actually, SP_EL0 is set separately — we use SPSR to configure SP_EL0
    // Set SP_EL0 via MSR before ERET isn't possible from here.
    // Instead, pass user_stack_top in x0 so user code can set SP itself,
    // OR we rely on the fact that SP_EL0 is a separate register.

    // Store user_stack_top in the frame at x0 position (sp+0)
    // and user_entry address in x1 (sp+8) for the user trampoline
    core::ptr::write_volatile(frame_base as *mut u64, user_stack_top as u64);

    frame_base
}

/// Create a new task
/// Returns task ID on success, None on failure
pub fn task_create(name: &'static str, func: extern "C" fn() -> !) -> Option<u32> {
    unsafe {
        // Disable IRQ during task creation (I bit = bit 2)
        core::arch::asm!("msr DAIFSet, #0x4");

        // Find free slot
        let mut slot = None;
        for i in 0..MAX_TASKS {
            if TASKS[i].is_none() || TASKS[i].as_ref().map(|t| t.state) == Some(TaskState::Dead) {
                slot = Some(i);
                break;
            }
        }

        let slot = match slot {
            Some(s) => s,
            None => {
                core::arch::asm!("msr DAIFClr, #0x4");
                return None;
            }
        };

        // Allocate stack and initialize frame
        let kernel_sp = init_task_stack(func);

        // Get task ID
        let id = TASK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

        // Create task (volatile copy to avoid TCG codegen hang)
        {
            let t = Some(Task {
                id,
                kernel_sp: kernel_sp as u64,
                state: TaskState::Ready,
                name,
                user_ttbr0: 0,
                user_sp: 0,
                is_user: false,
            });
            let src = &t as *const Option<Task> as *const u64;
            let dst = &mut TASKS[slot] as *mut Option<Task> as *mut u64;
            for i in 0..(core::mem::size_of::<Option<Task>>() / 8) {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }

        // Re-enable IRQ
        core::arch::asm!("msr DAIFClr, #0x4");

        Some(id)
    }
}

/// Create a new user (EL0) task
/// user_entry: virtual address of user code to execute
/// user_stack_size: size of user stack (e.g., 4096)
/// user_ttbr0: physical address of user page table (0 = identity map, no isolation)
/// Returns task ID on success, None on failure
#[allow(dead_code)]
pub fn task_create_user(name: &'static str, user_entry: usize, user_stack_size: usize, user_ttbr0: u64) -> Option<u32> {
    unsafe {
        core::arch::asm!("msr DAIFSet, #0x4");

        // Find free slot
        let mut slot = None;
        for i in 0..MAX_TASKS {
            if TASKS[i].is_none() || TASKS[i].as_ref().map(|t| t.state) == Some(TaskState::Dead) {
                slot = Some(i);
                break;
            }
        }

        let slot = match slot {
            Some(s) => s,
            None => {
                core::arch::asm!("msr DAIFClr, #0x4");
                return None;
            }
        };

        // Allocate kernel stack (for exception handling)
        let kernel_layout = Layout::from_size_align(STACK_SIZE, 16).unwrap();
        let kernel_stack_base = alloc_zeroed(kernel_layout) as usize;

        // Allocate user stack
        let user_layout = Layout::from_size_align(user_stack_size, 16).unwrap();
        let user_stack_base = alloc::alloc::alloc_zeroed(user_layout) as usize;
        let user_stack_top = user_stack_base + user_stack_size;

        // Initialize kernel stack frame for ERET to EL0
        let kernel_sp = init_user_task_stack(kernel_stack_base, user_entry, user_stack_top);

        let id = TASK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;

        // Use volatile copy to avoid TCG codegen hang on large struct assignment
        {
            let t = Some(Task {
                id,
                kernel_sp: kernel_sp as u64,
                state: TaskState::Ready,
                name,
                user_ttbr0,
                user_sp: user_stack_top as u64,
                is_user: true,
            });
            let src = &t as *const Option<Task> as *const u64;
            let dst = &mut TASKS[slot] as *mut Option<Task> as *mut u64;
            for i in 0..(core::mem::size_of::<Option<Task>>() / 8) {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }

        core::arch::asm!("msr DAIFClr, #0x4");

        Some(id)
    }
}


/// Called from timer_tick() in interrupt.rs
/// Performs round-robin scheduling and sets up context switch
pub fn scheduler_tick() {
    unsafe {
        // Always clear NEW_TASK_SP first — if no switch is needed,
        // the IRQ handler will see 0 and skip the context switch
        NEW_TASK_SP = 0;

        let current = CURRENT;

        // Find next Ready task (round-robin)
        let mut next = current;
        for i in 1..=MAX_TASKS {
            let idx = (current + i) % MAX_TASKS;
            if let Some(ref task) = TASKS[idx] {
                if task.state == TaskState::Ready {
                    next = idx;
                    break;
                }
            }
        }

        // If no switch needed, return
        if next == current {
            return;
        }

        // Mark current as Ready (was Running)
        if let Some(ref mut task) = TASKS[current] {
            if task.state == TaskState::Running {
                task.state = TaskState::Ready;
            }
        }

        // Mark next as Running
        if let Some(ref mut task) = TASKS[next] {
            task.state = TaskState::Running;
        }

        // Set up globals for assembly context switch
        // OLD_TASK_SP_PTR points to current task's kernel_sp field
        if let Some(ref mut task) = TASKS[current] {
            OLD_TASK_SP_PTR = &mut task.kernel_sp as *mut u64 as usize;
        }

        // NEW_TASK_SP is the actual SP value to load
        if let Some(ref task) = TASKS[next] {
            NEW_TASK_SP = task.kernel_sp as usize;

            // Set user TTBR0/SP globals for assembly TTBR0 switching
            crate::mmu::CURRENT_USER_TTBR0 = task.user_ttbr0;
            crate::mmu::CURRENT_USER_SP = task.user_sp;
        }

        CURRENT = next;
    }
}

/// Initialize task subsystem
/// Creates idle task (task 0) and registers current execution as task 1 (shell)
pub fn init() {
    // Debug: print key sizes and addresses
    // (removed — TCG hang risk)
    unsafe {
        let idle_sp = init_task_stack(idle_task);

        // Use volatile writes for TASKS entries to avoid TCG codegen hang
        {
            let t0 = Some(Task {
                id: 0,
                kernel_sp: idle_sp as u64,
                state: TaskState::Ready,
                name: "idle",
                user_ttbr0: 0,
                user_sp: 0,
                is_user: false,
            });
            let src = &t0 as *const Option<Task> as *const u64;
            let dst = &mut TASKS[0] as *mut Option<Task> as *mut u64;
            for i in 0..(core::mem::size_of::<Option<Task>>() / 8) {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }

        {
            let t1 = Some(Task {
                id: 1,
                kernel_sp: 0,
                state: TaskState::Running,
                name: "shell",
                user_ttbr0: 0,
                user_sp: 0,
                is_user: false,
            });
            let src = &t1 as *const Option<Task> as *const u64;
            let dst = &mut TASKS[1] as *mut Option<Task> as *mut u64;
            for i in 0..(core::mem::size_of::<Option<Task>>() / 8) {
                core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            }
        }

        CURRENT = 1;

        uart::puts(UART, "[OK] Task scheduler initialized\r\n");
    }
}

/// Get current task index
unsafe fn get_current_index() -> usize {
    CURRENT
}

/// Mark current task as Dead and yield forever. Called by tasks that want to exit.
pub fn task_exit() -> ! {
    unsafe {
        let idx = get_current_index();
        if let Some(ref mut task) = TASKS[idx] {
            task.state = TaskState::Dead;
            uart::puts(UART, "[OK] Task '");
            uart::puts(UART, task.name);
            uart::puts(UART, "' exited\r\n");
        }
    }
    loop {
        unsafe { core::arch::asm!("wfi"); }
    }
}

/// Kill a task by name. Returns true if found and killed.
#[allow(dead_code)]
pub fn task_kill_by_name(name: &str) -> bool {
    unsafe {
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASKS[i] {
                if task.name == name && task.state != TaskState::Dead {
                    TASKS[i].as_mut().map(|t| t.state = TaskState::Dead);
                    return true;
                }
            }
        }
    }
    false
}

/// Get current task ID
#[allow(dead_code)]
pub fn get_current_id() -> u32 {
    unsafe {
        TASKS[CURRENT].as_ref().map(|t| t.id).unwrap_or(0)
    }
}

/// Print task list (for shell command)
#[inline(never)]
fn print_task(idx: usize) {
    unsafe {
        let task_ptr = &TASKS[idx] as *const Option<Task>;
        // Read raw bytes to check if Some (name pointer non-null)
        let first_word = core::ptr::read_volatile(task_ptr as *const u64);
        if first_word == 0 { return; } // None — name ptr is null
        if let Some(ref task) = TASKS[idx] {
            if task.state == TaskState::Dead { return; }
            uart::puts(UART, "  ");
            uart::puts(UART, task.name);
            uart::puts(UART, "\r\n");
        }
    }
}

/// Print task list (for shell command)
pub fn print_task_list() {
    unsafe {
        uart::puts(UART, "Tasks:\r\n");
        for i in 0..MAX_TASKS {
            print_task(i);
        }
    }
}

// ─── IPC: Per-task Mailbox ──────────────────────────────────────────────────

/// Send data to a task's mailbox by name. Returns false if not found.
#[allow(dead_code)]
pub fn ipc_send(target_name: &str, data: &[u8]) -> bool {
    // Disable IRQ during access
    unsafe { core::arch::asm!("msr DAIFSet, #0x4"); }

    let result = unsafe {
        let mut found = false;
        for i in 0..MAX_TASKS {
            if let Some(ref task) = TASKS[i] {
                if task.name == target_name && task.state != TaskState::Dead {
                    let mb = &mut MAILBOXES[i];
                    for &b in data {
                        mb.push_back(b);
                    }
                    found = true;
                    break;
                }
            }
        }
        found
    };

    // Re-enable IRQ
    unsafe { core::arch::asm!("msr DAIFClr, #0x4"); }
    result
}

/// Receive data from current task's mailbox. Returns bytes read.
#[allow(dead_code)]
pub fn ipc_recv(buf: &mut [u8]) -> usize {
    // Disable IRQ during access
    unsafe { core::arch::asm!("msr DAIFSet, #0x4"); }

    let count = unsafe {
        let idx = CURRENT;
        let mb = &mut MAILBOXES[idx];
        let mut i = 0;
        while i < buf.len() {
            match mb.pop_front() {
                Some(b) => { buf[i] = b; i += 1; }
                None => break,
            }
        }
        i
    };

    // Re-enable IRQ
    unsafe { core::arch::asm!("msr DAIFClr, #0x4"); }
    count
}

/// Check if current task has mailbox data
#[allow(dead_code)]
pub fn ipc_has_data() -> bool {
    unsafe {
        let idx = CURRENT;
        !MAILBOXES[idx].is_empty()
    }
}

// ─── FD Table Operations ─────────────────────────────────────────────────────

/// Allocate an FD for the current task. Returns the FD index, or None if full.
pub fn fd_alloc(name: &[u8], flags: u32) -> Option<usize> {
    unsafe {
        let task_idx = CURRENT;
        for i in 0..MAX_FDS {
            let fd = &mut FD_TABLES[task_idx][i];
            if !fd.used {
                // Zero name
                for j in 0..FD_NAME_LEN { fd.name[j] = 0; }
                // Copy name
                let copy_len = core::cmp::min(name.len(), FD_NAME_LEN);
                for j in 0..copy_len {
                    fd.name[j] = name[j];
                }
                fd.offset = 0;
                fd.flags = flags;
                fd.used = true;
                return Some(i);
            }
        }
        None
    }
}

/// Get a mutable reference to an FD entry. Returns None if fd out of range or unused.
pub fn fd_get(fd_idx: usize) -> Option<*mut FdEntry> {
    unsafe {
        if fd_idx >= MAX_FDS { return None; }
        let task_idx = CURRENT;
        let fd = &mut FD_TABLES[task_idx][fd_idx];
        if fd.used { Some(fd) } else { None }
    }
}

/// Free (close) an FD for the current task.
pub fn fd_free(fd_idx: usize) -> bool {
    unsafe {
        if fd_idx >= MAX_FDS { return false; }
        let task_idx = CURRENT;
        let fd = &mut FD_TABLES[task_idx][fd_idx];
        if fd.used {
            fd.used = false;
            true
        } else {
            false
        }
    }
}
