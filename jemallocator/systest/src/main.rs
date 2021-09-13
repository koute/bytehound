#![allow(bad_style, improper_ctypes, dead_code, unused_imports)]

use std::alloc::System;

#[global_allocator]
static A: System = System;

use libc::{c_char, c_int, c_void};
use tikv_jemalloc_sys::*;

include!(concat!(env!("OUT_DIR"), "/all.rs"));
