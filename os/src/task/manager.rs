//!Implementation of [`TaskManager`]
use super::{TaskControlBlock, BIG_STRIDE};
use crate::sync::UPSafeCell;
use alloc::collections::BinaryHeap;
use alloc::sync::Arc;
use core::cmp::Ordering;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: BinaryHeap<StrideEntry>,//使用堆来组织task
}

/// 改成用堆的管理器了
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: BinaryHeap::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        let stride = {
            task.inner_exclusive_access().stride
        };
        let entry = StrideEntry::new(stride, task);
        self.ready_queue.push(entry);//这个现在是堆的push
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

//新添加这个数据结构,不破坏原来的TaskControlBlock,这样写二叉堆更清晰
struct StrideEntry {
    stride: usize,
    task: Arc<TaskControlBlock>,
}

impl StrideEntry {
    fn new(stride: usize, task: Arc<TaskControlBlock>) -> Self {
        Self { stride, task }
    }
}

impl PartialEq for StrideEntry {
    fn eq(&self, other: &Self) -> bool {
        self.stride == other.stride
    }
}

impl Eq for StrideEntry {}

//定义偏序
impl PartialOrd for StrideEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let diff = self.stride.wrapping_sub(other.stride);
        if diff == 0 {
            // 题目假设不会出现完全相等的 stride，这里仅防御性返回 Equal
            return Some(Ordering::Equal);
        }
        // 若 diff <= BIG_STRIDE / 2, 说明 self.stride >= other.stride (没有溢出)。
        // 我们希望 BinaryHeap 弹出真实最小的 stride，因此需要把“更小”当成“更大”。
        if diff <= BIG_STRIDE / 2 {
            Some(Ordering::Less)
        } else {
            Some(Ordering::Greater)
        }
    }
}

impl Ord for StrideEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}
