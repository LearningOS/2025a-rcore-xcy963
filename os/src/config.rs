//! Constants in the kernel

#[allow(unused)]

/// user app's stack size
pub const USER_STACK_SIZE: usize = 4096 * 2;
/// kernel stack size
pub const KERNEL_STACK_SIZE: usize = 4096 * 2;
/// kernel heap size
pub const KERNEL_HEAP_SIZE: usize = 0x200_0000;

/// page size : 4KB
pub const PAGE_SIZE: usize = 0x1000;
/// page size bits: 12
pub const PAGE_SIZE_BITS: usize = 0xc;
/// the max number of syscall
pub const MAX_SYSCALL_NUM: usize = 500;
/// the virtual addr of trapoline
/// 这个是0xffff_ffff_ffff_f000,如果把他理解成虚拟地址,我们来复习一下Sv39(低39位有效)
/// 低地址区：[0x0000_0000_0000_0000, 0x0000_007f_ffff_ffff]
/// 高地址区：[0xffff_ff80_0000_0000, 0xffff_ffff_ffff_ffff]
pub const TRAMPOLINE: usize = usize::MAX - PAGE_SIZE + 1;//高地址最后一页
/// the virtual addr of trap context
pub const TRAP_CONTEXT_BASE: usize = TRAMPOLINE - PAGE_SIZE;
/// clock frequency
pub const CLOCK_FREQ: usize = 12500000;
/// the physical memory end
pub const MEMORY_END: usize = 0x88000000;
