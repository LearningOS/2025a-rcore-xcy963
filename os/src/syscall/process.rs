//! Process management syscalls
use crate::{
    mm::{translated_byte_buffer, PageTable, PTEFlags, PhysPageNum, VirtAddr},
    task::{
        change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_times,
        mmap as task_mmap, munmap as task_munmap, suspend_current_and_run_next,
    },
    // loader::{get_app_addr_range, get_user_stack_range},
    // task::{ exit_current_and_run_next, suspend_current_and_run_next},
    timer::get_time_us,
    // trace,
};

const TRACE_REQ_READ: usize = 0;
const TRACE_REQ_WRITE: usize = 1;
const TRACE_REQ_SYSCALL: usize = 2;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    // trace!("kernel: sys_get_time");
    // -1
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let timeval = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let len = core::mem::size_of::<TimeVal>();
    let buffers = translated_byte_buffer(current_user_token(), _ts as *const u8, len);
    let src_bytes = unsafe {
        core::slice::from_raw_parts(&timeval as *const _ as *const u8, len)
    };
    let mut offset = 0;
    for buffer in buffers {
        let end = offset + buffer.len();
        buffer.copy_from_slice(&src_bytes[offset..end]);
        offset = end;
    }
    0

}

/// HINT: You might reimplement it with virtual memory management.
/// 需要做三件事情:读一块内存,写一快内存,还有返回当前应用执行某个系统调用的次数
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {//返回有时候是-1,所以是有符号的
    trace!("kernel: sys_trace");
    match trace_request {
        TRACE_REQ_READ => {
            trace_try_read(current_user_token(), id).map(|byte| byte as isize).unwrap_or(-1)
        }
        TRACE_REQ_WRITE => {
            if trace_try_write(current_user_token(), id, data as u8) {
                0
            } else {
                -1
            }
        }
        TRACE_REQ_SYSCALL => get_syscall_times(id) as isize,
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
//参数说明:
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    task_mmap(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    task_munmap(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

fn trace_translate_addr(token: usize, addr: usize) -> Option<(PhysPageNum, usize, PTEFlags)> {
    let page_table = PageTable::from_token(token);
    let vaddr = VirtAddr::from(addr);
    let vpn = vaddr.floor();
    let entry = page_table.translate(vpn)?;
    let flags = entry.flags();
    if !flags.contains(PTEFlags::U) {
        return None;
    }
    Some((entry.ppn(), vaddr.page_offset(), flags))
}

fn trace_try_read(token: usize, addr: usize) -> Option<u8> {
    trace_translate_addr(token, addr).and_then(|(ppn, offset, flags)| {
        if flags.contains(PTEFlags::R) {
            let bytes = ppn.get_bytes_array();
            Some(bytes[offset])
        } else {
            None
        }
    })
}

fn trace_try_write(token: usize, addr: usize, value: u8) -> bool {
    if let Some((ppn, offset, flags)) = trace_translate_addr(token, addr) {
        if flags.contains(PTEFlags::W) {
            let bytes = ppn.get_bytes_array();
            bytes[offset] = value;
            return true;
        }
    }
    false
}
