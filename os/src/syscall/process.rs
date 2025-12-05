//! Process management syscalls
use crate::{
    // loader::{get_app_addr_range, get_user_stack_range},
    task::{ exit_current_and_run_next, suspend_current_and_run_next},
    timer::get_time_us,
    trace,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

//trace的系统调用的主函数
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    //默认是寄存器直接传进来,所以类型都是usize
    trace!("kernel: sys_trace");
    match trace_request {
        0 => trace_read_byte(id)
            .map(|value| value as isize)
            .unwrap_or(-1),//如果读取的东西有问题,返回-1
        1 => {
            if trace_write_byte(id, data as u8) {
                0
            } else {
                -1
            }
        }
        2 => trace::current_task_syscall_count(id) as isize,
        _ => -1,
    }
}


fn trace_read_byte(addr: usize) -> Option<u8> {
    //从id处读取一个字节的无符号整数指
    Some(unsafe { core::ptr::read_volatile(addr as *const u8) })
}

fn trace_write_byte(addr: usize, data: u8) -> bool {

    unsafe { core::ptr::write_volatile(addr as *mut u8, data) };
    true
}
