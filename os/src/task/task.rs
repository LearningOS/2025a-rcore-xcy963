//! Types related to task management & Functions for completely changing TCB

use super::id::TaskUserRes;
use super::{kstack_alloc, KernelStack, ProcessControlBlock, TaskContext};
use crate::trap::TrapContext;
use crate::{mm::PhysPageNum, sync::UPSafeCell};
use alloc::sync::{Arc, Weak};
use core::cell::RefMut;
use core::cmp;

/// Large stride constant used for stride scheduling.
pub const BIG_STRIDE: usize = 1 << 20;
/// Default priority assigned to new tasks.
pub const DEFAULT_PRIORITY: usize = 16;
/// Minimum allowed priority for stride scheduling.
pub const MIN_PRIORITY: usize = 2;

/// Task control block structure
pub struct TaskControlBlock {//task没有id了...
    /// immutable
    pub process: Weak<ProcessControlBlock>,
    /// Kernel stack corresponding to PID
    pub kstack: KernelStack,
    /// mutable
    inner: UPSafeCell<TaskControlBlockInner>,
}

impl TaskControlBlock {
    /// Get the mutable reference of the inner TCB
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// Get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        let process = self.process.upgrade().unwrap();
        let inner = process.inner_exclusive_access();
        inner.memory_set.token()
    }
}

pub struct TaskControlBlockInner {
    pub res: Option<TaskUserRes>,
    /// The physical page number of the frame where the trap context is placed
    pub trap_cx_ppn: PhysPageNum,
    /// Save task context
    pub task_cx: TaskContext,

    /// Maintain the execution status of the current process
    pub task_status: TaskStatus,
    /// It is set when active exit or execution error occurs
    pub exit_code: Option<i32>,
    /// Stride scheduling priority (bigger => more CPU time)
    pub priority: usize,
    /// Current accumulated stride
    pub stride: usize,
    /// Amount to add to stride whenever task is scheduled
    pub stride_pass: usize,
}

impl TaskControlBlockInner {
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }

    #[allow(unused)]
    fn get_status(&self) -> TaskStatus {
        self.task_status
    }
    fn refresh_stride_pass(&mut self) {
        self.stride_pass = stride_pass_from_priority(self.priority);
    }
    pub fn set_priority(&mut self, priority: usize) {
        self.priority = priority;
        self.refresh_stride_pass();
    }
    pub fn add_stride(&mut self) {
        self.stride = self.stride.saturating_add(self.stride_pass);
    }
}

impl TaskControlBlock {
    /// Create a new task
    pub fn new(
        process: Arc<ProcessControlBlock>,
        ustack_base: usize,
        alloc_user_res: bool,
    ) -> Self {
        let res = TaskUserRes::new(Arc::clone(&process), ustack_base, alloc_user_res);
        let trap_cx_ppn = res.trap_cx_ppn();
        let kstack = kstack_alloc();
        let kstack_top = kstack.get_top();
        Self {
            process: Arc::downgrade(&process),
            kstack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    res: Some(res),
                    trap_cx_ppn,
                    task_cx: TaskContext::goto_trap_return(kstack_top),
                    task_status: TaskStatus::Ready,
                    exit_code: None,
                    priority: DEFAULT_PRIORITY,
                    stride: 0,
                    stride_pass: stride_pass_from_priority(DEFAULT_PRIORITY),
                })
            },
        }
    }
}

fn stride_pass_from_priority(priority: usize) -> usize {
    let prio = cmp::max(priority, 1);
    cmp::max(1, BIG_STRIDE / prio)
}

#[derive(Copy, Clone, PartialEq)]
/// The execution status of the current process
pub enum TaskStatus {
    /// ready to run
    Ready,
    /// running
    Running,
    /// blocked
    Blocked,
}
