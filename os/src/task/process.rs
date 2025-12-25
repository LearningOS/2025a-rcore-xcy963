//! Implementation of  [`ProcessControlBlock`]

use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, SignalFlags};
use super::{pid_alloc, PidHandle};
use crate::config::USER_STACK_SIZE;
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MapPermission, MemorySet, VirtAddr, KERNEL_SPACE};
use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use crate::trap::{trap_handler, TrapContext};
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

/// Process Control Block
pub struct ProcessControlBlock {
    /// immutable
    pub pid: PidHandle,
    /// mutable
    inner: UPSafeCell<ProcessControlBlockInner>,
}

/// Inner of Process Control Block
pub struct ProcessControlBlockInner {
    /// is zombie?
    pub is_zombie: bool,
    /// memory set(address space)
    pub memory_set: MemorySet,
    /// parent process
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// children process
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// exit code
    pub exit_code: i32, //变为zombi的时候回收读取
    /// file descriptor table
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    /// signal flags
    pub signals: SignalFlags,
    /// tasks(also known as threads)
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    /// task resource allocator
    pub task_res_allocator: RecycleAllocator,
    /// mutex list
    pub mutex_list: Vec<Option<Arc<dyn Mutex>>>,
    /// semaphore list
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// condvar list
    pub condvar_list: Vec<Option<Arc<Condvar>>>,
    /// deadlock detection flag
    pub deadlock_detect: bool,
    /// resource availability for deadlock detection
    pub dl_available: Vec<usize>,
    /// allocation matrix[tid][res] for deadlock detection
    pub dl_allocation: Vec<Vec<usize>>,
    /// pending need matrix[tid][res] for deadlock detection
    pub dl_need: Vec<Vec<usize>>,
    /// resource index mapping for mutex id
    pub dl_mutex_res: Vec<Option<usize>>, //为了拓展性,对其他资源建立一个链表,可以查到对应的资源
    /// resource index mapping for semaphore id
    pub dl_sem_res: Vec<Option<usize>>,
    /// mmap regions
    pub mmap_areas: BTreeMap<usize, MmapRegion>,
    /// program break lower bound
    pub heap_bottom: usize,
    /// current program break
    pub program_brk: usize,
    // pub dead_lock:bool,
}
///测试map的
#[derive(Clone)]
pub struct MmapRegion {
    /// Region start address, page aligned
    pub start: usize,
    /// Region length in bytes, already rounded up to page size
    pub len: usize,
    /// Permission used when mapping
    pub perm: MapPermission,
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    /// get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// allocate a new file descriptor
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
    /// allocate a new task id
    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }
    /// deallocate a task id
    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }
    /// the count of tasks(threads) in this process
    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }
    /// get a task with tid in this process
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }

    pub(crate) fn ensure_dl_task(&mut self, tid: usize) {
        //声明为内部函数
        let cols = self.dl_available.len();
        if tid >= self.dl_allocation.len() {
            self.dl_allocation.resize(tid + 1, Vec::new());
            self.dl_need.resize(tid + 1, Vec::new());
        }
        if self.dl_allocation[tid].len() < cols {
            self.dl_allocation[tid].resize(cols, 0);
        }
        if self.dl_need[tid].len() < cols {
            self.dl_need[tid].resize(cols, 0);
        }
    }

    pub(crate) fn add_resource(&mut self, cap: usize) -> usize {
        self.dl_available.push(cap);
        for row in &mut self.dl_allocation {
            //刚开始创建的时候每个线程都还没拥有这个资源
            row.push(0);
        }
        for row in &mut self.dl_need {
            //刚开始的时候每个线程不需要这个资源???
            row.push(cap);
        }
        self.dl_available.len() - 1
    }

    fn is_dl_safe(&self) -> bool {
        let mut work = self.dl_available.clone();
        let mut finish = vec![false; self.dl_allocation.len()];
        let empty: Vec<usize> = Vec::new();
        loop {
            let mut progress = false; //如果这一轮没有任何更新,那么算法结束,我们已经不能完成更多的进程
            for i in 0..self.dl_allocation.len() {
                //遍历所有的进程
                if finish[i] {
                    continue;
                }
                let row_need = self.dl_need.get(i).unwrap_or(&empty);
                if row_need
                    .iter()
                    .enumerate()
                    .all(|(j, n)| *n <= *work.get(j).unwrap_or(&0))
                {
                    for (j, a) in self.dl_allocation[i].iter().enumerate() {
                        if j < work.len() {
                            //更新work,假设释放这个线程所有的资源
                            work[j] += a;
                        }
                    }
                    finish[i] = true;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        finish.into_iter().all(|f| f)
    }

    pub(crate) fn dl_try_request(&mut self, tid: usize, res_idx: usize) -> bool {
        self.ensure_dl_task(tid);
        self.dl_allocation[tid][res_idx] += 1;
        self.dl_available[res_idx] = self.dl_available[res_idx].saturating_sub(1);
        self.dl_need[tid][res_idx] -= 1;
        let safe = self.is_dl_safe();
        if !safe {
            self.dl_need[tid][res_idx] += 1;
            self.dl_allocation[tid][res_idx] -= 1;
            self.dl_available[res_idx] += 1;
        }
        safe
    }

    // pub(crate) fn dl_finish_request(&mut self, tid: usize, res_idx: usize) {
    //     self.ensure_dl_task(tid);
    //     self.dl_need[tid][res_idx] = self.dl_need[tid][res_idx].saturating_sub(1);
    //     self.dl_available[res_idx] = self.dl_available[res_idx].saturating_sub(1);
    //     self.dl_allocation[tid][res_idx] += 1;
    // }

    pub(crate) fn dl_release_resource(&mut self, tid: usize, res_idx: usize) {
        self.ensure_dl_task(tid);
        if self.dl_allocation[tid][res_idx] > 0 {
            self.dl_allocation[tid][res_idx] -= 1;
        }
        self.dl_available[res_idx] += 1;
    }
}

impl ProcessControlBlock {
    /// inner_exclusive_access
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// new process from elf file
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        trace!("kernel: ProcessControlBlock::new");
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (mut memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let heap_bottom = ustack_base + USER_STACK_SIZE;
        memory_set.insert_framed_area(
            heap_bottom.into(),
            heap_bottom.into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );
        // allocate a pid
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    deadlock_detect: false,
                    dl_available: Vec::new(),
                    dl_allocation: Vec::new(),
                    dl_need: Vec::new(),
                    dl_mutex_res: Vec::new(),
                    dl_sem_res: Vec::new(),
                    mmap_areas: BTreeMap::new(),
                    heap_bottom,
                    program_brk: heap_bottom,
                })
            },
        });
        // create a main thread, we should allocate ustack and trap_cx here
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            true,
        ));
        // prepare trap_cx of main thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    /// Only support processes with a single thread.
    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        trace!("kernel: exec");
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        // memory_set with elf program headers/trampoline/trap context/user stack
        trace!("kernel: exec .. MemorySet::from_elf");
        let (mut memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let heap_bottom = ustack_base + USER_STACK_SIZE;
        memory_set.insert_framed_area(
            heap_bottom.into(),
            heap_bottom.into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );
        let new_token = memory_set.token();
        // substitute memory_set
        trace!("kernel: exec .. substitute memory_set");
        {
            let mut process_inner = self.inner_exclusive_access();
            process_inner.memory_set = memory_set;
            process_inner.mmap_areas.clear();
            process_inner.heap_bottom = heap_bottom;
            process_inner.program_brk = heap_bottom;
        }
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        trace!("kernel: exec .. alloc user resource for main thread again");
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        task_inner.res.as_mut().unwrap().ustack_base = ustack_base;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        trace!("kernel: exec .. push arguments on user stack");
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        trace!("kernel: exec .. initialize trap_cx");
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        trace!("kernel: fork");
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        let parent_main = parent.get_task(0);
        let (parent_priority, parent_stride, heap_bottom, program_brk) = {
            let inner = parent_main.inner_exclusive_access();
            (
                inner.priority,
                inner.stride,
                parent.heap_bottom,
                parent.program_brk,
            )
        };
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_existed_user(&parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    is_zombie: false,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    signals: SignalFlags::empty(),
                    tasks: Vec::new(),
                    task_res_allocator: RecycleAllocator::new(),
                    mutex_list: Vec::new(),
                    semaphore_list: Vec::new(),
                    condvar_list: Vec::new(),
                    //
                    deadlock_detect: parent.deadlock_detect,
                    dl_available: parent.dl_available.clone(),
                    dl_allocation: parent.dl_allocation.clone(),
                    dl_need: parent.dl_need.clone(),
                    dl_mutex_res: parent.dl_mutex_res.clone(),
                    dl_sem_res: parent.dl_sem_res.clone(),
                    mmap_areas: parent.mmap_areas.clone(),
                    heap_bottom,
                    program_brk,
                })
            },
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        {
            let mut task_inner = task.inner_exclusive_access();
            task_inner.set_priority(parent_priority);
            task_inner.stride = parent_stride;
        }
        // attach task to child process
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    /// get pid
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    /// change the location of the program break. return None if failed.
    pub fn change_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner_exclusive_access();
        let heap_bottom = inner.heap_bottom;
        let old_break = inner.program_brk;
        let new_brk = (inner.program_brk as isize).checked_add(size as isize)?;
        if new_brk < heap_bottom as isize {
            return None;
        }
        let new_brk = new_brk as usize;
        let ok = if size < 0 {
            inner
                .memory_set
                .shrink_to(VirtAddr::from(heap_bottom), VirtAddr::from(new_brk))
        } else {
            inner
                .memory_set
                .append_to(VirtAddr::from(heap_bottom), VirtAddr::from(new_brk))
        };
        if ok {
            inner.program_brk = new_brk;
            Some(old_break)
        } else {
            None
        }
    }
}
