use std::mem;
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::atomic::{self, AtomicUsize, Ordering};

struct Inner<T> {
    counter: AtomicUsize,
    value: T,
}

pub struct ArcLite<T>(NonNull<Inner<T>>);
unsafe impl<T> Send for ArcLite<T> where T: Sync + Send {}
unsafe impl<T> Sync for ArcLite<T> where T: Sync + Send {}

impl<T> ArcLite<T> {
    pub fn new(value: T) -> Self {
        let ptr = Box::into_raw(Box::new(Inner {
            counter: AtomicUsize::new(1),
            value,
        }));

        unsafe { ArcLite(NonNull::new_unchecked(ptr)) }
    }

    pub fn ptr_eq(lhs: &Self, rhs: &Self) -> bool {
        lhs.0 == rhs.0
    }

    #[inline]
    unsafe fn counter(arc: &Self) -> &AtomicUsize {
        &arc.0.as_ref().counter
    }

    #[inline]
    pub fn get_refcount_relaxed(arc: &Self) -> usize {
        unsafe { Self::counter(arc).load(Ordering::Relaxed) }
    }

    #[inline(never)]
    unsafe fn drop_slow(arc: &mut Self) {
        mem::drop(Box::from_raw(arc.0.as_ptr()));
    }

    pub unsafe fn add(arc: &Self, value: usize) {
        Self::counter(arc).fetch_add(value, Ordering::SeqCst);
    }

    pub unsafe fn sub(arc: &Self, value: usize) {
        Self::counter(arc).fetch_sub(value, Ordering::SeqCst);
    }
}

impl<T> Deref for ArcLite<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &unsafe { self.0.as_ref() }.value
    }
}

impl<T> Clone for ArcLite<T> {
    #[inline]
    fn clone(&self) -> Self {
        unsafe {
            ArcLite::counter(self).fetch_add(1, Ordering::Relaxed);
        }
        ArcLite(self.0)
    }
}

impl<T> Drop for ArcLite<T> {
    #[inline]
    fn drop(&mut self) {
        unsafe {
            if ArcLite::counter(self).fetch_sub(1, Ordering::Release) != 1 {
                return;
            }

            atomic::fence(Ordering::Acquire);
            ArcLite::drop_slow(self);
        }
    }
}
