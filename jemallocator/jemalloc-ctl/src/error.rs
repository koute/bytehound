//! Error type
#![cfg_attr(
    feature = "cargo-clippy",
    allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)
)]

use crate::{fmt, num, result};
use libc::c_int;

pub trait NonZeroT {
    type T;
}
impl NonZeroT for i32 {
    type T = num::NonZeroU32;
}
impl NonZeroT for i64 {
    type T = num::NonZeroU64;
}

pub type NonZeroCInt = <c_int as NonZeroT>::T;

/// Errors of the `tikv_jemalloc_sys::mallct`-family of functions.
///
/// The `jemalloc-sys` crate: `mallctl`, `mallctlnametomib`, and `mallctlbymib``
/// functions return `0` on success; otherwise they return an error value.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq)]
pub struct Error(NonZeroCInt);

/// Result type
pub type Result<T> = result::Result<T, Error>;

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let code = self.0.get() as c_int;
        match description(code) {
            Some(m) => write!(f, "{}", m),
            None => write!(f, "Unknown error code: \"{}\".", code),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

#[cfg(feature = "use_std")]
use std::error::Error as StdError;

#[cfg(feature = "use_std")]
impl StdError for Error {
    fn description(&self) -> &str {
        match description(self.0.get() as c_int) {
            Some(m) => m,
            None => "Unknown error",
        }
    }
    fn cause(&self) -> Option<&dyn StdError> {
        None
    }
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

fn description(code: c_int) -> Option<&'static str> {
    match code {
        libc::EINVAL => Some(
            "`newp` is not `NULL`, and `newlen` is too large or too \
             small. Alternatively, `*oldlenp` is too large or too \
             small; in this case as much data as possible are read \
             despite the error.",
        ),
        libc::ENOENT => {
            Some("`name` or `mib` specifies an unknown/invalid value.")
        }
        libc::EPERM => Some(
            "Attempt to read or write `void` value, or attempt to \
             write read-only value.",
        ),
        libc::EAGAIN => Some("A memory allocation failure occurred."),
        libc::EFAULT => Some(
            "An interface with side effects failed in some way not \
             directly related to `mallctl*()` read/write processing.",
        ),
        _ => None,
    }
}

pub(crate) fn cvt(ret: c_int) -> Result<()> {
    match ret {
        0 => Ok(()),
        v => Err(Error(unsafe { NonZeroCInt::new_unchecked(v as _) })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_of_result_error() {
        use crate::mem::size_of;
        assert_eq!(size_of::<Result<()>>(), size_of::<Error>());
        assert_eq!(size_of::<Error>(), size_of::<libc::c_int>());
    }
}
