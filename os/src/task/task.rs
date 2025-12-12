//! Types related to task management
use super::TaskContext;
use crate::config::{PAGE_SIZE, TRAP_CONTEXT_BASE};
use crate::mm::{
    kernel_stack_position, MapPermission, MemorySet, PhysPageNum, VirtAddr, VirtPageNum,
    KERNEL_SPACE,
};
use crate::trap::{trap_handler, TrapContext};
use alloc::collections::BTreeMap;

struct MmapRegion {
    len: usize,
    start_vpn: VirtPageNum,
    end_vpn: VirtPageNum,
}

/// The task control block (TCB) of a task.
pub struct TaskControlBlock {
    /// Save task context
    pub task_cx: TaskContext,

    /// Maintain the execution status of the current process
    pub task_status: TaskStatus,

    /// Application address space
    pub memory_set: MemorySet,

    /// The phys page number of trap context
    pub trap_cx_ppn: PhysPageNum,

    /// The size(top addr) of program which is loaded from elf file
    pub base_size: usize,

    /// Heap bottom
    pub heap_bottom: usize,

    /// Program break
    pub program_brk: usize,

    /// Syscall invocation statistics keyed by syscall id
    syscall_times: BTreeMap<usize, usize>,

    /// 实现mmap用的数据结构,键是va
    mmap_areas: BTreeMap<usize, MmapRegion>,
}

impl TaskControlBlock {
    /// get the trap context
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }
    /// get the user token
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// Based on the elf info in program, build the contents of task in a new address space
    pub fn new(elf_data: &[u8], app_id: usize) -> Self {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();//这个地方使用了一个约定,trap的代码一定是放在虚拟内存的最高位的,所以直接使用这个程序的页表映射最高位的虚拟地址就好
        let task_status = TaskStatus::Ready;
        // map a kernel-stack in kernel space
        let (kernel_stack_bottom, kernel_stack_top) = kernel_stack_position(app_id);
        KERNEL_SPACE.exclusive_access().insert_framed_area(//这一段是需要回收的,所以是framed
            kernel_stack_bottom.into(),
            kernel_stack_top.into(),
            MapPermission::R | MapPermission::W,
        );
        let task_control_block = Self {
            task_status,
            task_cx: TaskContext::goto_trap_return(kernel_stack_top),
            memory_set,
            trap_cx_ppn,
            base_size: user_sp,
            heap_bottom: user_sp,
            program_brk: user_sp,//现在的堆指针
            syscall_times: BTreeMap::new(),
            mmap_areas: BTreeMap::new(),
        };
        // prepare TrapContext in user space
        //认为应用刚刚初始化的时候是从trap过去的,走的是trap return的路
        let trap_cx = task_control_block.get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            kernel_stack_top,
            trap_handler as usize,//设置trap的回调函数
        );
        task_control_block
    }
    /// Increase syscall `id` invocation count for current task
    pub fn inc_syscall_times(&mut self, id: usize) {
        let counter = self.syscall_times.entry(id).or_insert(0);
        *counter += 1;
    }
    /// Get syscall invocation count for `id`
    pub fn get_syscall_times(&self, id: usize) -> usize {
        *self.syscall_times.get(&id).unwrap_or(&0)
    }
    /// Create a new mmap region
    pub fn mmap(&mut self, start: usize, len: usize, prot: usize) -> isize {
        if len == 0 || start & (PAGE_SIZE - 1) != 0 {
            return -1;
        }
        if prot == 0 || (prot & !0b111) != 0 {
            return -1;
        }
        if self.mmap_areas.contains_key(&start) {
            return -1;
        }
        let len_aligned = match Self::align_len(len) {
            Some(val) => val,
            None => return -1,
        };
        let end = match start.checked_add(len_aligned) {
            Some(end) => end,
            None => return -1,
        };
        let start_va = VirtAddr::from(start);
        let end_va = VirtAddr::from(end);
        let start_vpn = start_va.floor();
        let end_vpn = end_va.ceil();
        if start_vpn.0 >= end_vpn.0 {
            return -1;
        }
        println!("[kernal] debug xcy start:{:#x},end:{:#x}",start_vpn.0,end_vpn.0);
        for vpn in start_vpn.0..end_vpn.0 {
            if let Some(entry) = self.memory_set.translate(VirtPageNum(vpn)) {
                if entry.is_valid() {//已经有映射的话就报错,不能做多余的事情
                    return -1;
                }
            }
        }
        let mut perm = MapPermission::U;
        if prot & 0x1 != 0 {
            perm |= MapPermission::R;
        }
        if prot & 0x2 != 0 {
            perm |= MapPermission::W;
        }
        if prot & 0x4 != 0 {
            perm |= MapPermission::X;
        }
        if perm == MapPermission::U {
            return -1;
        }
        self.memory_set
            .insert_framed_area(start_va, end_va, perm);
        // 记录我们做过的mmap段,之后可以清除
        self.mmap_areas.insert(
            start,
            MmapRegion {
                len,
                start_vpn,
                end_vpn,
            },
        );
        0
    }
    /// Remove a previously created mmap region
    pub fn munmap(&mut self, start: usize, len: usize) -> isize {
        if len == 0 || start & (PAGE_SIZE - 1) != 0 {
            return -1;
        }
        if Self::align_len(len).is_none() {
            return -1;
        }
        if let Some(region) = self.mmap_areas.get(&start) {
            if region.len != len {
                return -1;
            }
            if self
                .memory_set
                .remove_framed_area(region.start_vpn, region.end_vpn)
            {
                self.mmap_areas.remove(&start);
                0
            } else {
                -1
            }
        } else {
            -1
        }
    }
    #[inline]
    fn align_len(len: usize) -> Option<usize> {
        if len == 0 {
            return None;
        }
        len.checked_add(PAGE_SIZE - 1)
            .map(|val| val & !(PAGE_SIZE - 1))
    }
    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&mut self, size: i32) -> Option<usize> {
        let old_break = self.program_brk;
        let new_brk = self.program_brk as isize + size as isize;
        if new_brk < self.heap_bottom as isize {
            return None;
        }
        let result = if size < 0 {
            self.memory_set
                .shrink_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        } else {
            self.memory_set
                .append_to(VirtAddr(self.heap_bottom), VirtAddr(new_brk as usize))
        };
        if result {
            self.program_brk = new_brk as usize;
            Some(old_break)
        } else {
            None
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Exited
pub enum TaskStatus {
    /// uninitialized
    UnInit,
    /// ready to run
    Ready,
    /// running
    Running,
    /// exited
    Exited,
}
