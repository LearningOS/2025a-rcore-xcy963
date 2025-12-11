//! Implementation of [`TrapContext`]
use riscv::register::sstatus::{self, Sstatus, SPP};

#[repr(C)]
#[derive(Debug)]
/// trap context structure containing sstatus, sepc and registers
pub struct TrapContext {
    /// General-Purpose Register x0-31
    pub x: [usize; 32],
    /// Supervisor Status Register
    pub sstatus: Sstatus,
    /// Supervisor Exception Program Counter
    pub sepc: usize,
    /// Token of kernel address space
    pub kernel_satp: usize,//Supervisor Address Translation and Protection Register
    /// Kernel stack pointer of the current application
    pub kernel_sp: usize,
    /// Virtual address of trap handler entry point in kernel
    pub trap_handler: usize,
}

impl TrapContext {
    /// put the sp(stack pointer) into x\[2\] field of TrapContext
    pub fn set_sp(&mut self, sp: usize) {
        self.x[2] = sp;
    }
    /// init the trap context of an application
    pub fn app_init_context(
        entry: usize,//程序段入口地址,这个是一个虚拟地址,编写应用程序的人是知道的
        sp: usize,//用户栈的虚拟地址
        kernel_satp: usize,//内核的虚拟地址空间状态,也就是页表的树根地址
        kernel_sp: usize,  //这个程序对应的内核栈的虚拟地址
        trap_handler: usize,//TODOtrap回调函数的物理地址,这个传入的是在当前rust程序中的地址,应该是物理地址,那么为什么他可以在虚拟地址环境下正常工作?
    ) -> Self {
        let mut sstatus = sstatus::read();//复制内核的sstatus,这个是描述trap的时候是用户态还是内核态的寄存器
        // set CPU privilege to User after trapping back
        sstatus.set_spp(SPP::User);
        let mut cx = Self {
            x: [0; 32],
            sstatus,
            sepc: entry,  // entry point of app
            kernel_satp,  // addr of page table
            kernel_sp,    // kernel stack
            trap_handler, // addr of trap_handler function
        };
        cx.set_sp(sp); // app's user stack pointer
        cx // return initial Trap Context of app
    }
}
