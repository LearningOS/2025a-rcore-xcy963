//! Process management syscalls
use alloc::sync::Arc;

use crate::{
    config::{PAGE_SIZE, TRAP_CONTEXT_BASE},
    loader::get_app_data_by_name,
    mm::{
        translated_byte_buffer, translated_refmut, translated_str, MapPermission, VirtAddr,
        VirtPageNum,
    },
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, MmapRegion,
    },
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
/// 还是一样,需要向地址_ts里面写上
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    if ts.is_null() {//首先字面值要有数据
        return -1;
    }
    let time_us = get_time_us();
    let timeval = TimeVal {
        sec: time_us / 1_000_000,
        usec: time_us % 1_000_000,
    };
    let len = core::mem::size_of::<TimeVal>();
    let src = unsafe {
        core::slice::from_raw_parts((&timeval as *const TimeVal) as *const u8, len)
    };
    let mut copied = 0usize;
    let mut buffers = translated_byte_buffer(current_user_token(), ts as *const u8, len);
    for buffer in buffers.iter_mut() {
        if copied >= len {
            break;
        }
        let copy_len = buffer.len().min(len - copied);
        buffer[..copy_len].copy_from_slice(&src[copied..copied + copy_len]);
        copied += copy_len;
    }
    if copied == len { 0 } else { -1 }
}


//一个上取整函数
fn align_up_len(len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let addend = PAGE_SIZE - 1;
    let aligned = len.checked_add(addend)?;
    Some(aligned / PAGE_SIZE * PAGE_SIZE)
}
//计算权限
fn prot_to_perm(prot: usize) -> Option<MapPermission> {
    const ALLOWED: usize = 0x7;
    if prot == 0 || prot & !ALLOWED != 0 {
        return None;
    }
    let mut perm = MapPermission::U;
    if prot & 0x1 != 0 {
        perm |= MapPermission::R;
    }
    if prot & 0x2 != 0 {
        perm |= MapPermission::W | MapPermission::R;
    }
    if prot & 0x4 != 0 {
        perm |= MapPermission::X;
    }
    Some(perm)
}

/// YOUR JOB: Implement mmap.
/// mmap是
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    if start % PAGE_SIZE != 0 {
        return -1;
    }
    let len_aligned = match align_up_len(len) {
        Some(l) => l,
        None => return -1,
    };
    let end = match start.checked_add(len_aligned) {
        Some(e) => e,
        None => return -1,
    };
    if end > TRAP_CONTEXT_BASE {//用户要太多了
        return -1;
    }
    let perm = match prot_to_perm(prot) {
        Some(p) => p,
        None => return -1,
    };
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if inner.mmap_areas.contains_key(&start) {
        return -1;
    }
    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(end).ceil();
    let mut vpn = start_vpn.0;
    while vpn < end_vpn.0 {//页表中有了,那么就需要返回错误
        if inner
            .memory_set
            .translate(VirtPageNum(vpn))
            .is_some()
        {
            return -1;
        }
        vpn += 1;
    }
    inner
        .memory_set
        .insert_framed_area(VirtAddr::from(start), VirtAddr::from(end), perm);
    inner.mmap_areas.insert(
        start,
        MmapRegion {
            start,
            len: len_aligned,
            perm,
        },
    );
    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    if start % PAGE_SIZE != 0 || len == 0 || len % PAGE_SIZE != 0 {
        return -1;
    }
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if let Some(region) = inner.mmap_areas.get(&start) {
        if len != region.len {
            return -1;
        }
    } else {
        return -1;
    }
    inner
        .memory_set
        .remove_area_with_start_vpn(VirtAddr::from(start).into());
    inner.mmap_areas.remove(&start);
    0
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
/// 实现思路:直接向队列里面添加一个,类似最早的时候添加第一个任务一样
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}
