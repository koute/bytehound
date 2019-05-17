use std::ffi::CStr;
use libc;

pub fn open( path: &CStr, flags: libc::c_int, mode: libc::c_int ) -> libc::c_int {
    let path = path.as_ptr();

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
