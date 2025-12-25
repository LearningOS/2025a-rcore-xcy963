//! Implementation of [`TaskManager`]
//!
//! It is only used to manage processes and schedule process based on ready queue.
//! Other CPU process monitoring functions are in Processor.

use super::{ProcessControlBlock, TaskControlBlock, TaskStatus, BIG_STRIDE};
use crate::sync::UPSafeCell;
use alloc::collections::{BTreeMap, BinaryHeap};
use alloc::sync::Arc;
use core::cmp::Ordering;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: BinaryHeap<StrideEntry>,

    /// The stopping task, leave a reference so that the kernel stack will not be recycled when switching tasks
    /// 当主线程退出的时候,需要把task的指针保留一份ARC的在这里,不然rust会把他回收
    stop_task: Option<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: BinaryHeap::new(),
            stop_task: None,
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        let entry = StrideEntry::new(task);
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
    pub fn remove(&mut self, task: Arc<TaskControlBlock>) {
        let target_ptr = Arc::as_ptr(&task);
        let mut new_heap = BinaryHeap::new();
        while let Some(entry) = self.ready_queue.pop() {
            if Arc::as_ptr(&entry.task) != target_ptr {
                new_heap.push(entry);
            }
        }
        self.ready_queue = new_heap;
    }
    /// Add a task to stopping task
    pub fn add_stop(&mut self, task: Arc<TaskControlBlock>) {
        // NOTE: as the last stopping task has completely stopped (not
        // using kernel stack any more, at least in the single-core
        // case) so that we can simply replace it;
        self.stop_task = Some(task);
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
    /// PID2PCB instance (map of pid to pcb)
    pub static ref PID2PCB: UPSafeCell<BTreeMap<usize, Arc<ProcessControlBlock>>> =
        unsafe { UPSafeCell::new(BTreeMap::new()) };
}

/// Add a task to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Wake up a task
pub fn wakeup_task(task: Arc<TaskControlBlock>) {
    trace!("kernel: TaskManager::wakeup_task");
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    add_task(task);
}

/// Remove a task from the ready queue
pub fn remove_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::remove_task");
    TASK_MANAGER.exclusive_access().remove(task);
}

/// Fetch a task out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}

// A thin wrapper to keep the stride info in the scheduling heap.
struct StrideEntry {
    stride: usize,
    id: usize,//由于要适配线程,所以task不维护id了,我们使用task的地址来管理等于的情况
    task: Arc<TaskControlBlock>,
}

impl StrideEntry {
    fn new(task: Arc<TaskControlBlock>) -> Self {
        let stride = { task.inner_exclusive_access().stride };
        let id = Arc::as_ptr(&task) as usize;
        Self { stride, id, task }
    }
}

impl PartialEq for StrideEntry {
    fn eq(&self, other: &Self) -> bool {
        self.stride == other.stride && self.id == other.id
    }
}

impl Eq for StrideEntry {}

impl PartialOrd for StrideEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let diff: isize = (self.stride as isize) - (other.stride as isize);
        if diff == 0 {
            return Some(self.id.cmp(&other.id));
        }
        let diff_abs = diff.unsigned_abs();
        if diff_abs <= BIG_STRIDE / 2 {
            if diff < 0 {
                Some(Ordering::Greater)
            } else {
                Some(Ordering::Less)
            }
        } else if diff > 0 {
            Some(Ordering::Greater)
        } else {
            Some(Ordering::Less)
        }
    }
}

impl Ord for StrideEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}

/// Set a task to stop-wait status, waiting for its kernel stack out of use.
pub fn add_stopping_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add_stop(task);
}

/// Get process by pid
pub fn pid2process(pid: usize) -> Option<Arc<ProcessControlBlock>> {
    let map = PID2PCB.exclusive_access();
    map.get(&pid).map(Arc::clone)
}

/// Insert item(pid, pcb) into PID2PCB map (called by do_fork AND ProcessControlBlock::new)
pub fn insert_into_pid2process(pid: usize, process: Arc<ProcessControlBlock>) {
    PID2PCB.exclusive_access().insert(pid, process);
}

/// Remove item(pid, _some_pcb) from PDI2PCB map (called by exit_current_and_run_next)
pub fn remove_from_pid2process(pid: usize) {
    let mut map = PID2PCB.exclusive_access();
    if map.remove(&pid).is_none() {
        panic!("cannot find pid {} in pid2task!", pid);
    }
}
