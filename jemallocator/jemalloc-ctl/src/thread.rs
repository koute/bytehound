//! Thread specific operations.

use crate::error::Result;
use crate::raw::{read, read_mib};

option! {
    allocatedp[ str: b"thread.allocatedp\0", non_str: 2 ] => *mut u64 |
    ops:  |
    docs:
    /// Access to the total number of bytes allocated by the current thread.
    ///
    /// Unlike [`::stats::allocated`], the value returned by this type is not the
    /// number of bytes *currently* allocated, but rather the number of bytes
    /// that have *ever* been allocated by this thread.
    ///
    /// The `read` method doesn't return the value directly, but actually a
    /// pointer to the value. This allows for very fast repeated lookup, since
    /// there is no function call overhead. The pointer type cannot be sent to
    /// other threads, but `allocated::read` can be called on different threads
    /// and will return the appropriate pointer for each of them.
    ///
    /// # Example
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::thread;
    /// let allocated = thread::allocatedp::mib().unwrap();
    /// let allocated = allocated.read().unwrap();
    ///
    /// let a = allocated.get();
    /// let buf = vec![0; 1024 * 1024];
    /// let b = allocated.get();
    /// drop(    buf);
    /// let c = allocated.get();
    ///
    /// assert!(a < b);
    /// assert_eq!(b, c);
    /// # }
    /// ```
    mib_docs: /// See [`allocatedp`].
}

impl allocatedp {
    /// Reads value using string API.
    pub fn read() -> Result<ThreadLocal<u64>> {
        unsafe { read(Self::name().as_bytes()).map(ThreadLocal) }
    }
}

impl allocatedp_mib {
    /// Reads value using MIB API.
    pub fn read(&self) -> Result<ThreadLocal<u64>> {
        unsafe { read_mib(self.0.as_ref()).map(ThreadLocal) }
    }
}

option! {
    deallocatedp[ str: b"thread.deallocatedp\0", non_str: 2 ] => *mut u64 |
    ops:  |
    docs:
    /// Access to the total number of bytes deallocated by the current thread.
    ///
    /// The `read` method doesn't return the value directly, but actually a
    /// pointer to the value. This allows for very fast repeated lookup, since
    /// there is no function call overhead. The pointer type cannot be sent to
    /// other threads, but [`deallocatedp::read`] can be called on different
    /// threads and will return the appropriate pointer for each of them.
    ///
    /// # Example
    ///
    /// ```
    /// # #[global_allocator]
    /// # static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
    /// #
    /// # fn main() {
    /// use tikv_jemalloc_ctl::thread;
    /// let deallocated = thread::deallocatedp::mib().unwrap();
    /// let deallocated = deallocated.read().unwrap();
    ///
    /// let a = deallocated.get();
    /// let buf = vec![0; 1024 * 1024];
    /// let b = deallocated.get();
    /// drop(buf);
    /// let c = deallocated.get();
    ///
    /// assert_eq!(a, b);
    /// assert!(b < c);
    /// # }
    /// ```
    mib_docs: /// See [`deallocatedp`].
}

impl deallocatedp {
    /// Reads value using string API.
    pub fn read() -> Result<ThreadLocal<u64>> {
        unsafe { read(Self::name().as_bytes()).map(ThreadLocal) }
    }
}

impl deallocatedp_mib {
    /// Reads value using MIB API.
    pub fn read(&self) -> Result<ThreadLocal<u64>> {
        unsafe { read_mib(self.0.as_ref()).map(ThreadLocal) }
    }
}

/// A thread-local pointer.
///
/// It is neither `Sync` nor `Send`.
// NB we need *const here specifically since it's !Sync + !Send
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct ThreadLocal<T>(*const T);

impl<T> ThreadLocal<T>
where
    T: Copy,
{
    /// Returns the current value at the pointer.
    #[inline]
    pub fn get(self) -> T {
        unsafe { *self.0 }
    }
}
