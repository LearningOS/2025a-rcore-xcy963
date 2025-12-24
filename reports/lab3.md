# ch5的报告

## 简单总结你实现的功能

### `sys_get_time` `sys_mmap` `sys_munmap`
> 基本延续上几章的做法,没有什么改变

### `spawn`
> 模仿最开始添加第一个进程的时候做的事情,创建一个新的进程的空间,然后把他加到最小堆里面(这里并没定义他的优先级等等,都是默认值)
> 但是有调整他的节点关系,设置了他的父亲,还有他的父亲也设置了对应的儿子 

### 调度算法

- 每次要从队列里面选出stride值最小的task,那么用堆来维护队列就很合适
  - 使用rust的BinaryHeap,改造比较的函数使得他成为最小堆,每次时钟中断,先push进去当前的进程,然后再取出堆顶的进程
  - 更新优先级主要是更改`TaskControlBlockInner`里面的priority和pass

## 问答题

**stride 算法原理非常简单，但是有一个比较大的问题。例如两个 pass = 10 的进程，使用 8bit 无符号整形储存 stride， p1.stride = 255, p2.stride = 250，在 p2 执行一个时间片后，理论上下一次应该 p1 执行。**

**实际情况是轮到 p1 执行吗？为什么？**
> 答:不是仍然是p2,p2在添加之后由于溢出p2.stride 变为4,p1需要等待好久

我们之前要求进程优先级 >= 2 其实就是为了解决这个问题。可以证明， 在不考虑溢出的情况下 , 在进程优先级全部 >= 2 的情况下，如果严格按照算法执行，那么 STRIDE_MAX – STRIDE_MIN <= BigStride / 2。

> 答:考虑使用数学归纳法证明
> 
> 假设我们的Stride可以无穷大,那么最开始的时候肯定是满足的(大家都是0),由于最大的pass <= BigStride / 2,
> 
> 不妨设当前Stride 最小的进程(记stride是stride_小)对应的pass是BigStride / 2,记Stride 最大的进程的Stride为Stride_大
> 
> 有Stride_大 <= stride_小 + BigStride / 2(由归纳假设得出)
>
> 那么可见这一轮加上pass之后,仍然有假设成立,所以命题得证
>

```rust
use core::cmp::Ordering;

struct Stride(u64);

impl PartialOrd for Stride {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let diff = self.0.wrapping_sub(other.0) as i64;

        if diff < 0 {
            Some(Ordering::Greater)
        } else {
            Some(Ordering::Less)
        }
    }
}

impl PartialEq for Stride {
    fn eq(&self, _other: &Self) -> bool {
        false 
    }
}


```

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

> 我的操作系统老师刘国军

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

> [linux0.00](https://github.com/rongwu/linux-0.00_)参考其中的思路,理解了用户态与内核态的跳转

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。