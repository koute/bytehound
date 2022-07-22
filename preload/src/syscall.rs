use std::ffi::CStr;
use libc;
use crate::utils::Buffer;

#[cfg(not(feature = "sc"))]
macro_rules! syscall {
    (@to_libc OPEN) => { libc::SYS_open };
    (@to_libc OPENAT) => { libc::SYS_openat };
    (@to_libc CLOSE) => { libc::SYS_close };
    (@to_libc WRITE) => { libc::SYS_write };
    (@to_libc UMASK) => { libc::SYS_umask };
    (@to_libc FCHMOD) => { libc::SYS_fchmod };
    (@to_libc RENAME) => { libc::SYS_rename };
    (@to_libc RENAMEAT) => { libc::SYS_renameat };
    (@to_libc GETTID) => { libc::SYS_gettid };
    (@to_libc EXIT) => { libc::SYS_exit };
    (@to_libc MMAP) => { libc::SYS_mmap };
    (@to_libc MMAP2) => { libc::SYS_mmap2 };
    (@to_libc MUNMAP) => { libc::SYS_munmap };
    (@to_libc GETPID) => { libc::SYS_getpid };

    ($num:ident) => {
        libc::syscall( syscall!( @to_libc $num ) )
    };

    ($num:ident, $($args:expr),+) => {
        libc::syscall( syscall!( @to_libc $num ), $($args),+ )
    };
}

pub fn open( path: &CStr, flags: libc::c_int, mode: libc::c_int ) -> libc::c_int {
    open_raw_cstr( path.as_ptr(), flags, mode )
}

pub fn open_raw_cstr( path: *const libc::c_char, flags: libc::c_int, mode: libc::c_int ) -> libc::c_int {
    #[cfg(not(target_arch = "aarch64"))]
    unsafe {
        syscall!( OPEN, path, flags, mode ) as _
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        syscall!( OPENAT, libc::AT_FDCWD, path, flags, mode ) as _
    }
}

pub fn close( fd: libc::c_int ) -> libc::c_int {
    unsafe {
        syscall!( CLOSE, fd ) as _
    }
}

pub fn write( fd: libc::c_int, buffer: &[u8] ) -> libc::ssize_t {
    unsafe {
        syscall!( WRITE, fd, buffer.as_ptr(), buffer.len() ) as _
    }
}

pub fn umask( umask: libc::c_int ) -> libc::c_int {
    unsafe {
        syscall!( UMASK, umask ) as _
    }
}

pub fn fchmod( fd: libc::c_int, mode: libc::mode_t ) -> libc::c_int {
    unsafe {
        syscall!( FCHMOD, fd, mode ) as _
    }
}

pub fn rename( source: &CStr, destination: &CStr ) -> libc::c_int {
    let source = source.as_ptr();
    let destination = destination.as_ptr();

    #[cfg(not(target_arch = "aarch64"))]
    unsafe {
        syscall!( RENAME, source, destination ) as _
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        syscall!( RENAMEAT, libc::AT_FDCWD, source, libc::AT_FDCWD, destination ) as _
    }
}

pub fn getpid() -> libc::pid_t {
    unsafe {
        syscall!( GETPID ) as libc::pid_t
    }
}

pub fn gettid() -> u32 {
    unsafe {
        syscall!( GETTID ) as u32
    }
}

pub fn exit( status: u32 ) -> ! {
    unsafe {
        syscall!( EXIT, status );
        core::hint::unreachable_unchecked();
    }
}

#[cfg(target_arch = "arm")]
pub unsafe fn mmap( addr: *mut libc::c_void, length: libc::size_t, prot: libc::c_int, flags: libc::c_int, fildes: libc::c_int, off: libc::off_t ) -> *mut libc::c_void {
    syscall!( MMAP2, addr, length, prot, flags, fildes, off / (crate::PAGE_SIZE as libc::off_t) ) as *mut libc::c_void
}

#[cfg(not(target_arch = "arm"))]
pub unsafe fn mmap( addr: *mut libc::c_void, length: libc::size_t, prot: libc::c_int, flags: libc::c_int, fildes: libc::c_int, off: libc::off_t ) -> *mut libc::c_void {
    syscall!( MMAP, addr, length, prot, flags, fildes, off ) as *mut libc::c_void
}

pub unsafe fn munmap( addr: *mut libc::c_void, length: libc::size_t ) -> libc::c_int {
    syscall!( MUNMAP, addr, length ) as libc::c_int
}

extern "C" {
    static __environ: *const *const u8;
}

pub unsafe fn getenv( key: &[u8] ) -> Option< Buffer > {
    let mut current = __environ;
    loop {
        let p = *current;
        if p.is_null() {
            return None;
        }

        let mut r = p;
        while *r != b'=' {
            r = r.add( 1 );
        }

        let entry_key_length = r as usize - p as usize;
        let entry_key = std::slice::from_raw_parts( p, entry_key_length );
        if key == entry_key {
            r = r.add( 1 );
            let value_pointer = r;
            while *r != 0 {
                r = r.add( 1 );
            }

            let value_length = r as usize - value_pointer as usize;
            return Buffer::from_slice( std::slice::from_raw_parts( value_pointer, value_length ) );
        }

        current = current.add( 1 );
    }
}

#[test]
fn test_getenv() {
    std::env::set_var( "GETENV_TEST_VAR", "1234" );
    assert_eq!( unsafe { getenv( b"GETENV_TEST_VAR" ) }.unwrap().to_str().unwrap(), "1234" );
}