//! Benchmarks the cost of the different allocation functions by doing a
//! roundtrip (allocate, deallocate).
#![feature(test, allocator_api)]
#![cfg(feature = "alloc_trait")]

extern crate jemalloc_sys;
extern crate jemallocator;
extern crate libc;
extern crate paste;
extern crate test;

use jemalloc_sys::MALLOCX_ALIGN;
use jemallocator::Jemalloc;
use libc::c_int;
use std::{
    alloc::{Alloc, Excess, Layout},
    ptr,
};
use test::Bencher;

#[global_allocator]
static A: Jemalloc = Jemalloc;

// FIXME: replace with jemallocator::layout_to_flags
#[cfg(all(any(
    target_arch = "arm",
    target_arch = "mips",
    target_arch = "mipsel",
    target_arch = "powerpc"
)))]
const MIN_ALIGN: usize = 8;
#[cfg(all(any(
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "powerpc64",
    target_arch = "powerpc64le",
    target_arch = "mips64",
    target_arch = "s390x",
    target_arch = "sparc64"
)))]
const MIN_ALIGN: usize = 16;

fn layout_to_flags(layout: &Layout) -> c_int {
    if layout.align() <= MIN_ALIGN && layout.align() <= layout.size() {
        0
    } else {
        MALLOCX_ALIGN(layout.align())
    }
}

macro_rules! rt {
    ($size:expr, $align:expr) => {
        paste::item! {
            #[bench]
            fn [<rt_mallocx_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    use jemalloc_sys as jemalloc;
                    let flags = layout_to_flags(&Layout::from_size_align($size, $align).unwrap());
                    let ptr = jemalloc::mallocx($size, flags);
                    test::black_box(ptr);
                    jemalloc::sdallocx(ptr, $size, flags);
                });
            }

            #[bench]
            fn [<rt_mallocx_nallocx_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    use jemalloc_sys as jemalloc;
                    let flags = layout_to_flags(&Layout::from_size_align($size, $align).unwrap());
                    let ptr = jemalloc::mallocx($size, flags);
                    test::black_box(ptr);
                    let rsz = jemalloc::nallocx($size, flags);
                    test::black_box(rsz);
                    jemalloc::sdallocx(ptr, rsz, flags);
                });
            }

            #[bench]
            fn [<rt_alloc_layout_checked_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);
                    Jemalloc.dealloc(ptr, layout);
                });
            }

            #[bench]
            fn [<rt_alloc_layout_unchecked_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align_unchecked($size, $align);
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);
                    Jemalloc.dealloc(ptr, layout);
                });
            }

            #[bench]
            fn [<rt_alloc_excess_unused_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let Excess(ptr, _) = Jemalloc.alloc_excess(layout.clone()).unwrap();
                    test::black_box(ptr);
                    Jemalloc.dealloc(ptr, layout);
                });
            }

            #[bench]
            fn [<rt_alloc_excess_used_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let Excess(ptr, excess) = Jemalloc.alloc_excess(layout.clone()).unwrap();
                    test::black_box(ptr);
                    test::black_box(excess);
                    Jemalloc.dealloc(ptr, layout);
                });
            }

            #[bench]
            fn [<rt_mallocx_zeroed_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    use jemalloc_sys as jemalloc;
                    let flags = layout_to_flags(&Layout::from_size_align($size, $align).unwrap());
                    let ptr = jemalloc::mallocx($size, flags | jemalloc::MALLOCX_ZERO);
                    test::black_box(ptr);
                    jemalloc::sdallocx(ptr, $size, flags);
                });
            }

            #[bench]
            fn [<rt_calloc_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    use jemalloc_sys as jemalloc;
                    let flags = layout_to_flags(&Layout::from_size_align($size, $align).unwrap());
                    test::black_box(flags);
                    let ptr = jemalloc::calloc(1, $size);
                    test::black_box(ptr);
                    jemalloc::sdallocx(ptr, $size, 0);
                });
            }

            #[bench]
            fn [<rt_realloc_naive_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);

                    // navie realloc:
                    let new_layout = Layout::from_size_align(2 * $size, $align).unwrap();
                    let ptr = {
                        let new_ptr = Jemalloc.alloc(new_layout.clone()).unwrap();
                        ptr::copy_nonoverlapping(ptr.as_ptr() as *const u8, new_ptr.as_ptr(), layout.size());
                        Jemalloc.dealloc(ptr, layout);
                        new_ptr
                    };
                    test::black_box(ptr);

                    Jemalloc.dealloc(ptr, new_layout);
                });
            }

            #[bench]
            fn [<rt_realloc_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);

                    let new_layout = Layout::from_size_align(2 * $size, $align).unwrap();
                    let ptr = Jemalloc.realloc(ptr, layout, new_layout.size()).unwrap();
                    test::black_box(ptr);

                    Jemalloc.dealloc(ptr, new_layout);
                });
            }

            #[bench]
            fn [<rt_realloc_excess_unused_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);

                    let new_layout = Layout::from_size_align(2 * $size, $align).unwrap();
                    let Excess(ptr, _) = Jemalloc
                        .realloc_excess(ptr, layout, new_layout.size())
                        .unwrap();
                    test::black_box(ptr);

                    Jemalloc.dealloc(ptr, new_layout);
                });
            }

            #[bench]
            fn [<rt_realloc_excess_used_size_ $size _align_ $align>](b: &mut Bencher) {
                b.iter(|| unsafe {
                    let layout = Layout::from_size_align($size, $align).unwrap();
                    let ptr = Jemalloc.alloc(layout.clone()).unwrap();
                    test::black_box(ptr);

                    let new_layout = Layout::from_size_align(2 * $size, $align).unwrap();
                    let Excess(ptr, excess) = Jemalloc
                        .realloc_excess(ptr, layout, new_layout.size())
                        .unwrap();
                    test::black_box(ptr);
                    test::black_box(excess);

                    Jemalloc.dealloc(ptr, new_layout);
                });
            }

        }
    };
    ([$($size:expr),*]) => {
        $(
            rt!($size, 1);
            rt!($size, 2);
            rt!($size, 4);
            rt!($size, 8);
            rt!($size, 16);
            rt!($size, 32);
        )*
    }
}

// Powers of two
mod pow2 {
    use super::*;

    rt!([
        1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072,
        4194304
    ]);

}

mod even {
    use super::*;

    rt!([10, 100, 1000, 10000, 100000, 1000000]);
}

mod odd {
    use super::*;
    rt!([9, 99, 999, 9999, 99999, 999999]);
}

mod primes {
    use super::*;
    rt!([
        3, 7, 13, 17, 31, 61, 96, 127, 257, 509, 1021, 2039, 4093, 8191, 16381, 32749, 65537,
        131071, 4194301
    ]);
}
