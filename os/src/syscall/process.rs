use crate::timer::get_time_us;
use crate::{
    config::{PAGE_SIZE, TRAP_CONTEXT_BASE},
    fs::{open_file, OpenFlags},
    mm::{
        translated_byte_buffer, translated_ref, translated_refmut, translated_str, MapPermission,
        VirtAddr, VirtPageNum,
    },
    task::{
        current_process, current_task, current_user_token, exit_current_and_run_next, pid2process,
        suspend_current_and_run_next, MmapRegion, ProcessControlBlock, SignalFlags, MIN_PRIORITY,
    },
};
use alloc::{string::String, sync::Arc, vec::Vec};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// exit syscall
///
/// exit the current task and run the next task in task list
pub fn sys_exit(exit_code: i32) -> ! {
    trace!(
        "kernel:pid[{}] sys_exit",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}
/// yield syscall
pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}
/// getpid syscall
pub fn sys_getpid() -> isize {
    trace!(
        "kernel: sys_getpid pid:{}",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    current_task().unwrap().process.upgrade().unwrap().getpid() as isize
}
/// fork child process syscall
pub fn sys_fork() -> isize {
    trace!(
        "kernel:pid[{}] sys_fork",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.getpid();
    // modify trap context of new_task, because it returns immediately after switching
    let new_process_inner = new_process.inner_exclusive_access();
    let task = new_process_inner.tasks[0].as_ref().unwrap();
    let trap_cx = task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    new_pid as isize
}
/// exec syscall
pub fn sys_exec(path: *const u8, mut args: *const usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_exec",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    let token = current_user_token();
    let path = translated_str(token, path);
    let mut args_vec: Vec<String> = Vec::new();
    loop {
        let arg_str_ptr = *translated_ref(token, args);
        if arg_str_ptr == 0 {
            break;
        }
        args_vec.push(translated_str(token, arg_str_ptr as *const u8));
        unsafe {
            args = args.add(1);
        }
    }
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let process = current_process();
        let argc = args_vec.len();
        process.exec(all_data.as_slice(), args_vec);
        // return argc because cx.x[10] will be covered with it later
        argc as isize
    } else {
        -1
    }
}

/// waitpid syscall
///
/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let process = current_process();
    // find a child process

    let mut inner = process.inner_exclusive_access();
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
        p.inner_exclusive_access().is_zombie && (pid == -1 || pid as usize == p.getpid())
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

/// kill syscall
pub fn sys_kill(pid: usize, signal: u32) -> isize {
    trace!(
        "kernel:pid[{}] sys_kill",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if let Some(process) = pid2process(pid) {
        if let Some(flag) = SignalFlags::from_bits(signal) {
            process.inner_exclusive_access().signals |= flag;
            0
        } else {
            -1
        }
    } else {
        -1
    }
}

/// get_time syscall
///
/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if ts.is_null() {
        return -1;
    }
    let time_us = get_time_us();
    let timeval = TimeVal {
        sec: time_us / 1_000_000,
        usec: time_us % 1_000_000,
    };
    let len = core::mem::size_of::<TimeVal>();
    let src =
        unsafe { core::slice::from_raw_parts((&timeval as *const TimeVal) as *const u8, len) };
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
    if copied == len {
        0
    } else {
        -1
    }
}

/// mmap syscall
///
/// YOUR JOB: Implement mmap.
fn align_up_len(len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let addend = PAGE_SIZE - 1;
    let aligned = len.checked_add(addend)?;
    Some(aligned / PAGE_SIZE * PAGE_SIZE)
}

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

pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
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
    if end > TRAP_CONTEXT_BASE {
        return -1;
    }
    let perm = match prot_to_perm(prot) {
        Some(p) => p,
        None => return -1,
    };
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if inner.mmap_areas.contains_key(&start) {
        return -1;
    }
    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(end).ceil();
    let mut vpn = start_vpn.0;
    while vpn < end_vpn.0 {
        let mapped = inner
            .memory_set
            .translate(VirtPageNum(vpn))
            .map(|pte| pte.is_valid())
            .unwrap_or(false);
        if mapped {
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

/// munmap syscall
///
/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if start % PAGE_SIZE != 0 || len == 0 || len % PAGE_SIZE != 0 {
        return -1;
    }
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
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
    trace!(
        "kernel:pid[{}] sys_sbrk",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if let Some(old_brk) = current_process().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// spawn syscall
/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    let caller = current_task().unwrap();
    trace!(
        "kernel:pid[{}] sys_spawn",
        caller.process.upgrade().unwrap().getpid()
    );
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let data = inode.read_all();
        let parent = caller.process.upgrade().unwrap();
        let new_process = ProcessControlBlock::new(data.as_slice());
        {
            let mut child_inner = new_process.inner_exclusive_access();
            child_inner.parent = Some(Arc::downgrade(&parent));
        }
        {
            let mut parent_inner = parent.inner_exclusive_access();
            parent_inner.children.push(new_process.clone());
        }
        new_process.getpid() as isize
    } else {
        -1
    }
}

/// set priority syscall
///
/// YOUR JOB: Set task priority
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority",
        current_task().unwrap().process.upgrade().unwrap().getpid()
    );
    if prio < MIN_PRIORITY as isize {
        return -1;
    }
    let priority = prio as usize;
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.set_priority(priority);
    prio
}
