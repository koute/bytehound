#![allow(bad_style, improper_ctypes, dead_code, unused_imports)]
#![feature(global_allocator, allocator_api)]

extern crate jemalloc_sys;
extern crate libc;

use std::alloc::System;

#[global_allocator]
static A: System = System;

use libc::{c_int};
use jemalloc_sys::*;

include!(concat!(env!("OUT_DIR"), "/all.rs"));
