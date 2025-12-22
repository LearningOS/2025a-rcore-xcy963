//!Implementation of [`TaskManager`]
use super::{TaskControlBlock,BIG_STRIDE};
use crate::sync::UPSafeCell;
use alloc::collections::BinaryHeap;
use alloc::sync::Arc;
use core::cmp::Ordering;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: BinaryHeap<StrideEntry>,
}

/// Stride scheduler powered by a binary heap.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: BinaryHeap::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        let stride = { task.inner_exclusive_access().stride };
        let pid = task.getpid();//pid是唯一的,除非进程被回收
        let entry = StrideEntry::new(stride, pid, task);
        self.ready_queue.push(entry);
    }
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.ready_queue.pop().map(|entry| {
            let task = entry.task;
            {
                let mut inner = task.inner_exclusive_access();
                inner.add_stride();
            }
            task
        })
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}

// A thin wrapper to keep the stride info in the scheduling heap.
struct StrideEntry {
    stride: usize,
    pid: usize,
    task: Arc<TaskControlBlock>,
}

impl StrideEntry {
    fn new(stride: usize, pid: usize, task: Arc<TaskControlBlock>) -> Self {
        Self { stride, pid, task }
    }
}

impl PartialEq for StrideEntry {
    fn eq(&self, other: &Self) -> bool {
        self.stride == other.stride && self.pid == other.pid
    }
}

impl Eq for StrideEntry {}

impl PartialOrd for StrideEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        // Reverse the order so the smallest stride gets popped first.
        // Some(
        //     other
        //         .stride
        //         .cmp(&self.stride)
        //         .then_with(|| other.pid.cmp(&self.pid)),
        // )
        let diff: isize = (self.stride as isize) - (other.stride as isize);
        if diff == 0 {
            if self.pid > other.pid {//这个地方使用了pid的唯一性
                return Some(Ordering::Greater)
            }else{
                return Some(Ordering::Less);
            }
        }
        let diff_abs = if diff>0{
            diff as usize
        }else{
            -diff as usize
        };
        // 若 diff <= BIG_STRIDE / 2, 说明 self.stride >= other.stride (没有溢出)。
        // 我们希望 BinaryHeap 弹出真实最小的 stride，因此需要把“更小”当成“更大”。
        if diff_abs <= BIG_STRIDE / 2 {
            if diff > 0{
                return Some(Ordering::Greater)
            }else{
                return Some(Ordering::Less)
            }
        } else {//超过一个环了
            if diff < 0{
                return Some(Ordering::Greater)
            }else{
                return Some(Ordering::Less)
            }
        }
    }
}

impl Ord for StrideEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}
