//! `Arc<Inode>` -> `OSInodeInner`: In order to open files concurrently
//! we need to wrap `Inode` into `Arc`,but `Mutex` in `Inode` prevents
//! file systems from being accessed simultaneously
//!
//! `UPSafeCell<OSInodeInner>` -> `OSInode`: for static `ROOT_INODE`,we
//! need to wrap `OSInodeInner` into `UPSafeCell`
use super::{File, Stat, StatMode};
use crate::drivers::BLOCK_DEVICE;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::*;
use easy_fs::{EasyFileSystem, Inode};
use lazy_static::*;

/// inode in memory
/// A wrapper around a filesystem inode
/// to implement File trait atop
pub struct OSInode {
    readable: bool,
    writable: bool,
    inner: UPSafeCell<OSInodeInner>,
}
/// The OS inode inner in 'UPSafeCell'
pub struct OSInodeInner {
    offset: usize,
    inode: Arc<Inode>,
}

impl OSInode {
    /// create a new inode in memory
    pub fn new(readable: bool, writable: bool, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }
    /// read all data from the inode
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buffer: Vec<u8> = Vec::with_capacity(512); //相当于reserve
        buffer.resize(512, 0); //相当于填充0
        let mut v: Vec<u8> = Vec::new();
        loop {
            //一次只读取512字节,也就是一次最多
            let len = inner.inode.read_at(inner.offset, &mut buffer); //读取512B
            if len == 0 {
                break;
            }
            inner.offset += len; //这个inode在读取结束之后是销毁的,所以直接修改了他的offset
            v.extend_from_slice(&buffer[..len]);
        }
        v
    }
}

lazy_static! {//使用一个btree来保存
    static ref LINK_COUNTS: UPSafeCell<BTreeMap<u32, u32>> =
        unsafe { UPSafeCell::new(BTreeMap::new()) };
}

fn init_nlink(inode_id: u32) {
    let mut map = LINK_COUNTS.exclusive_access();
    map.entry(inode_id).or_insert(1);
}

fn inc_nlink(inode_id: u32) -> u32 {
    let mut map = LINK_COUNTS.exclusive_access();
    let counter = map.entry(inode_id).or_insert(1);
    *counter += 1;
    *counter
}

fn dec_nlink(inode_id: u32) -> u32 {
    let mut map = LINK_COUNTS.exclusive_access();
    let counter = map.entry(inode_id).or_insert(1);
    if *counter > 0 {
        *counter -= 1;
    }
    *counter
}
///获取这个文件有多少个链接
pub fn get_nlink(inode_id: u32) -> u32 {
    let mut map = LINK_COUNTS.exclusive_access();
    let counter = map.entry(inode_id).or_insert(1);
    *counter
}

lazy_static! {
    pub static ref ROOT_INODE: Arc<Inode> = {
        let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
        Arc::new(EasyFileSystem::root_inode(&efs))
    };
}

/// List all apps in the root directory
pub fn list_apps() {
    println!("/**** APPS ****");
    for app in ROOT_INODE.ls() {
        println!("{}", app);
    }
    println!("**************/");
}

bitflags! {
    ///  The flags argument to the open() system call is constructed by ORing together zero or more of the following values:
    pub struct OpenFlags: u32 {
        /// readyonly
        const RDONLY = 0;
        /// writeonly
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 9;
        /// truncate file size to 0
        const TRUNC = 1 << 10;
    }
}

impl OpenFlags {
    /// Do not check validity for simplicity
    /// Return (readable, writable)
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

/// Open a file
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.read_write();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = ROOT_INODE.find(name) {
            // clear size
            inode.clear();
            init_nlink(inode.inode_id());
            Some(Arc::new(OSInode::new(readable, writable, inode)))
        } else {
            // create file
            ROOT_INODE.create(name).map(|inode| {
                init_nlink(inode.inode_id());
                Arc::new(OSInode::new(readable, writable, inode))
            })
        }
    } else {
        ROOT_INODE.find(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            init_nlink(inode.inode_id());
            Arc::new(OSInode::new(readable, writable, inode))
        })
    }
}

impl File for OSInode {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_read_size = 0usize;
        for slice in buf.buffers.iter_mut() {
            let read_size = inner.inode.read_at(inner.offset, *slice);
            if read_size == 0 {
                break;
            }
            inner.offset += read_size;
            total_read_size += read_size;
        }
        total_read_size
    }
    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_write_size = 0usize;
        for slice in buf.buffers.iter() {
            let write_size = inner.inode.write_at(inner.offset, *slice);
            assert_eq!(write_size, slice.len());
            inner.offset += write_size;
            total_write_size += write_size;
        }
        total_write_size
    }
    fn stat(&self, st: &mut Stat) -> isize {
        let inode = {
            let inner = self.inner.exclusive_access();
            inner.inode.clone()
        };
        let inode_id = inode.inode_id();
        st.dev = 0;//驱动器号,写死是0
        st.ino = inode_id as u64;
        st.mode = if inode.is_dir() {
            StatMode::DIR
        } else {
            StatMode::FILE
        };
        st.nlink = get_nlink(inode_id);
        0
    }
}

/// 根据文件的路径(字符串)创建链接
pub fn link_file(old: &str, new: &str) -> Option<()> {
    if old == new {
        return None;
    }
    let old_inode = ROOT_INODE.find(old)?;
    if ROOT_INODE.find(new).is_some() {//新的文件已经存在
        return None;
    }
    let inode_id = old_inode.inode_id();
    ROOT_INODE.link(new, inode_id)?;
    inc_nlink(inode_id);
    Some(())
}

/// 删除对应的目录项还有数据块
pub fn unlink_file(name: &str) -> Option<()> {
    let inode = ROOT_INODE.find(name)?;
    let inode_id = inode.inode_id();
    ROOT_INODE.unlink(name)?;
    dec_nlink(inode_id);
    Some(())
}
