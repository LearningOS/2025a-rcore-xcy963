# ch6的报告

## 简单总结你实现的功能



**正常内核就是维护三个数据结构,这些都是描述现在运行的资源分配的实际情况**
- `dl_available`:剩下的资源数量,一维
- `dl_allocation`:各个线程占有的资源情况,二维
- `dl_need`: 各个线程还需要多少资源,二维

### 当一个线程尝试获取资源的时候
- 1. 先分配这个资源给这个线程,
  - 更新三件套,dl_available对应的资源减少,dl_allocation变多,dl_need减少
- 2. 初始化work向量是`dl_available`,初始化`finish`向量,每个任务都是false
- 3. 循环查找能结束的线程,也就是`need<work`的部分
  - 找到之后假设释放这个线程所有的资源(也就是更新work为work+need)
- 4. 如果本轮循环不能再使得线程结束,那么开始遍历finish向量
  - 如果发现这个时候有线程不能结束,那么就说明这个资源申请会造成死锁,我们不能放行
  - 之后要把dl_available,dl_allocation,dl_need还原

### 关于内核对上述三件套的维护

> 这里把锁和信号量统称为资源,资源只是被分配一个usize来计数
- 1. 资源创建的时候,需要更新各个线程的need为最大值,新的资源的available为最大值,各个线程的alloc是0
- 2. 资源回收的时候,也需要把相应的资源清空,把数组内部对应的改成`none`

## 问答作业

### 1.在我们的多线程实现中，当主线程 (即 0 号线程) 退出时，视为整个进程退出， 此时需要结束该进程管理的所有线程并回收其资源。 - 需要回收的资源有哪些？ - 其他线程的 TaskControlBlock 可能在哪些位置被引用，分别是否需要回收，为什么？

- 关于要回收的资源
> 每个新创建的线程就是task,会被放在process的`tasks: Vec<Option<Arc<TaskControlBlock>>>,`中,其中线程独享的数据是trap_cx,tip,等等。共享的是地址空间,对应的内核栈等等。具体的回收代码是`exit_current_and_run_next`

- 其他线程的TaskControlBlock
> 我们只有在process的tasks保留了arc的引用,其他都是weak的指针,不保证生命周期的,所以是不需要回收的

### 2. 对比以下两种 Mutex 中的实现，二者有什么区别？这些区别可能会导致什么问题？

```rust
impl Mutex for Mutex1 {
    fn lock(&self) {
        loop {
            let mut mutex_inner = self.inner.exclusive_access();
            if mutex_inner.locked {
                mutex_inner.wait_queue.push_back(current_task().unwrap());
                drop(mutex_inner);
                block_current_and_run_next();
            } else {
                mutex_inner.locked = true;
                break;
            }
        }
    }
    
    fn unlock(&self) {
        let mut mutex_inner = self.inner.exclusive_access();
        assert!(mutex_inner.locked);
        mutex_inner.locked = false;
        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
            add_task(waking_task);
        }
    }
}

impl Mutex for Mutex2 {
    fn lock(&self) {
        let mut mutex_inner = self.inner.exclusive_access();
        if mutex_inner.locked {
            mutex_inner.wait_queue.push_back(current_task().unwrap());
            drop(mutex_inner);
            block_current_and_run_next();
        } else {
            mutex_inner.locked = true;
        }
    }

    fn unlock(&self) {
        let mut mutex_inner = self.inner.exclusive_access();
        assert!(mutex_inner.locked);
        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
            add_task(waking_task);
        } else {
            mutex_inner.locked = false;
        }
    }
}
```

- Mutex2会有问题,当一个线程请求锁之后,如果他不能获取锁,他是被加到等待队列里面,之后再次轮到他的时候他会直接执行,不会检查他是否能获取到锁
- 而如果使用Mutex1的写法,只有当他能获取到锁的时候他才会继续执行

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

> 我的操作系统老师刘国军

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

> [linux0.00](https://github.com/rongwu/linux-0.00_)参考其中的思路,理解了用户态与内核态的跳转

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。