//! Facilities for tracing and reporting syscall usage per task.
///
///考虑到需要实现统计当前任务的系统调用次数,添加一个数据结构
///简单写下思路,首先有一个数据结构,做一个映射f:(appid,syscallid)->使用的系统调用次数,这里选择B树
///首先肯定每次系统调用(所有的系统调用)都需要获取当前的app的id,还有系统调用的id,然后定位B树节点去加

use crate::{config::MAX_APP_NUM, sync::UPSafeCell, task};
use alloc::{collections::BTreeMap, vec::Vec};
use lazy_static::lazy_static;
//模仿TaskManager
struct TraceManager {
    syscall_counts: Vec<BTreeMap<usize, usize>>,//vec的下标是appid,因为appid是连续的,但是我们的syscall相当离散
}

impl TraceManager {
    fn new() -> Self {
        let mut syscall_counts = Vec::new();
        syscall_counts.resize_with(MAX_APP_NUM, BTreeMap::new);
        Self { syscall_counts }
    }

    fn record_syscall(&mut self, task_id: usize, syscall_id: usize) {
        if let Some(map) = self.syscall_counts.get_mut(task_id) {
            let counter = map.entry(syscall_id).or_insert(0);
            *counter += 1;
        }
    }

    fn get_syscall_count(&self, task_id: usize, syscall_id: usize) -> usize {
        self.syscall_counts
            .get(task_id)
            .and_then(|map| map.get(&syscall_id).copied())
            .unwrap_or(0)
    }
}

lazy_static! {
    static ref TRACE_MANAGER: UPSafeCell<TraceManager> =
        unsafe { UPSafeCell::new(TraceManager::new()) };
}

/// Record that current task has invoked `syscall_id`.
pub fn record_syscall(syscall_id: usize) {
    let task_id = task::current_task_id();
    TRACE_MANAGER
        .exclusive_access()
        .record_syscall(task_id, syscall_id);
}

/// 获取当前任务调用编号为 id 的系统调用的次数，返回值为这个调用次数。本次调用也计入统计 。
/// 开始的时候理解错误了~以为是所有的id是应用程序的id,我真傻,真的~
pub fn current_task_syscall_count(syscall_id: usize) -> usize {
    let task_id = task::current_task_id();
    TRACE_MANAGER
        .exclusive_access()
        .get_syscall_count(task_id, syscall_id)
}
