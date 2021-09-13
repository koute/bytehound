//! Key types to index the _MALLCTL NAMESPACE_.
//!
//! The [`Name`] and [`Mib`]/[`MibStr`] types are provided as safe indices into
//! the _MALLCTL NAMESPACE_. These are constructed from null-terminated strings
//! via the [`AsName`] trait. The [`Access`] trait provides provides safe access
//! into the `_MALLCTL NAMESPACE_`.
//!
//! # Example
//!
//! ```
//! #[global_allocator]
//! static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
//!
//! fn main() {
//!     use tikv_jemalloc_ctl::{Access, AsName, Name, Mib};
//!     use libc::{c_uint, c_char};
//!     let name = b"arenas.nbins\0".name();
//!     let nbins: c_uint = name.read().unwrap();
//!     let mut mib: Mib<[usize; 4]> = b"arenas.bin.0.size\0".name().mib().unwrap();
//!     for i in 0..4 {
//!         mib[2] = i;
//!         let bin_size: usize = mib.read().unwrap();
//!         println!("arena bin {} has size {}", i, bin_size);
//!     }
//! }
//! ```

use crate::error::Result;
use crate::std::str;
use crate::{fmt, ops, raw};

/// A `Name` in the _MALLCTL NAMESPACE_.
#[repr(transparent)]
#[derive(PartialEq)]
pub struct Name([u8]);

/// Converts a null-terminated byte-string into a [`Name`].
pub trait AsName {
    /// Converts a null-terminated byte-string into a [`Name`].
    fn name(&self) -> &Name;
}

impl AsName for [u8] {
    fn name(&self) -> &Name {
        assert!(
            !self.is_empty(),
            "cannot create Name from empty byte-string"
        );
        assert_eq!(
            *self.last().unwrap(),
            b'\0',
            "cannot create Name from non-null-terminated byte-string \"{}\"",
            str::from_utf8(self).unwrap()
        );
        unsafe { &*(self as *const Self as *const Name) }
    }
}

impl AsName for str {
    fn name(&self) -> &Name {
        self.as_bytes().name()
    }
}

impl Name {
    /// Returns the [`Mib`] of `self`.
    pub fn mib<T: MibArg>(&self) -> Result<Mib<T>> {
        let mut mib: Mib<T> = Mib::default();
        raw::name_to_mib(&self.0, mib.0.as_mut())?;
        Ok(mib)
    }

    /// Returns the [`MibStr`] of `self` which is a key whose value is a string.
    pub fn mib_str<T: MibArg>(&self) -> Result<MibStr<T>> {
        assert!(
            self.value_type_str(),
            "key \"{}\" does not refer to a string",
            self
        );
        let mut mib: MibStr<T> = MibStr::default();
        raw::name_to_mib(&self.0, mib.0.as_mut())?;
        Ok(mib)
    }

    /// Returns `true` if `self` is a key in the _MALLCTL NAMESPCE_ referring to
    /// a null-terminated string.
    pub fn value_type_str(&self) -> bool {
        // remove the null-terminator:
        let name = self.0.split_at(self.0.len() - 1).0;
        if name.is_empty() {
            return false;
        }
        debug_assert_ne!(*name.last().unwrap(), b'\0');

        match name {
            b"version"
            | b"config.malloc_conf"
            | b"opt.metadata_thp"
            | b"opt.dss"
            | b"opt.percpu_arena"
            | b"opt.stats_print_opts"
            | b"opt.junk"
            | b"opt.thp"
            | b"opt.prof_prefix"
            | b"thread.prof.name"
            | b"prof.dump" => true,
            v if v.starts_with(b"arena.") && v.ends_with(b".dss") => true,
            v if v.starts_with(b"stats.arenas.") && v.ends_with(b".dss") => {
                true
            }
            _ => false,
        }
    }

    /// Returns the name as null-terminated byte-string.
    pub fn as_bytes(&self) -> &'static [u8] {
        unsafe { &*(self as *const Self as *const [u8]) }
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", str::from_utf8(&self.0).unwrap())
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", str::from_utf8(&self.0).unwrap())
    }
}

/// Management Information Base of a non-string value.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct Mib<T: MibArg>(T);

/// Management Information Base of a string value.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub struct MibStr<T: MibArg>(T);

impl<T: MibArg> AsRef<[usize]> for Mib<T> {
    fn as_ref(&self) -> &[usize] {
        self.0.as_ref()
    }
}

impl<T: MibArg> AsMut<[usize]> for Mib<T> {
    fn as_mut(&mut self) -> &mut [usize] {
        self.0.as_mut()
    }
}

impl<T: MibArg> ops::Index<usize> for Mib<T> {
    type Output = usize;
    fn index(&self, idx: usize) -> &Self::Output {
        &self.0.as_ref()[idx]
    }
}

impl<T: MibArg> ops::IndexMut<usize> for Mib<T> {
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
        &mut self.0.as_mut()[idx]
    }
}

impl<T: MibArg> ops::Index<usize> for MibStr<T> {
    type Output = usize;
    fn index(&self, idx: usize) -> &Self::Output {
        &self.0.as_ref()[idx]
    }
}

impl<T: MibArg> ops::IndexMut<usize> for MibStr<T> {
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
        &mut self.0.as_mut()[idx]
    }
}

/// Safe read access to the _MALLCTL NAMESPACE_.
pub trait Access<T> {
    /// Read the key at `self`.
    fn read(&self) -> Result<T>;
    /// Write `value` at the key `self`.
    fn write(&self, value: T) -> Result<()>;
    /// Write `value` at the key `self` returning its previous value.
    fn update(&self, value: T) -> Result<T>;
}

macro_rules! impl_access {
    ($id:ty) => {
        impl<T: MibArg> Access<$id> for Mib<T> {
            fn read(&self) -> Result<$id> {
                unsafe { raw::read_mib(self.0.as_ref()) }
            }
            fn write(&self, value: $id) -> Result<()> {
                unsafe { raw::write_mib(self.0.as_ref(), value) }
            }
            fn update(&self, value: $id) -> Result<$id> {
                unsafe { raw::update_mib(self.0.as_ref(), value) }
            }
        }
        impl Access<$id> for Name {
            fn read(&self) -> Result<$id> {
                unsafe { raw::read(&self.0) }
            }
            fn write(&self, value: $id) -> Result<()> {
                unsafe { raw::write(&self.0, value) }
            }
            fn update(&self, value: $id) -> Result<$id> {
                unsafe { raw::update(&self.0, value) }
            }
        }
    };
}

impl_access!(u32);
impl_access!(u64);
impl_access!(isize);
impl_access!(usize);

impl<T: MibArg> Access<bool> for Mib<T> {
    fn read(&self) -> Result<bool> {
        unsafe {
            let v: u8 = raw::read_mib(self.0.as_ref())?;
            assert!(v == 0 || v == 1);
            Ok(v == 1)
        }
    }
    fn write(&self, value: bool) -> Result<()> {
        unsafe { raw::write_mib(self.0.as_ref(), value) }
    }
    fn update(&self, value: bool) -> Result<bool> {
        unsafe {
            let v: u8 = raw::update_mib(self.0.as_ref(), value as u8)?;
            Ok(v == 1)
        }
    }
}

impl Access<bool> for Name {
    fn read(&self) -> Result<bool> {
        unsafe {
            let v: u8 = raw::read(&self.0)?;
            assert!(v == 0 || v == 1);
            Ok(v == 1)
        }
    }
    fn write(&self, value: bool) -> Result<()> {
        unsafe { raw::write(&self.0, value) }
    }
    fn update(&self, value: bool) -> Result<bool> {
        unsafe {
            let v: u8 = raw::update(&self.0, value as u8)?;
            Ok(v == 1)
        }
    }
}

impl<T: MibArg> Access<&'static [u8]> for MibStr<T> {
    fn read(&self) -> Result<&'static [u8]> {
        // this is safe because the only safe way to construct a `MibStr` is by
        // validating that the key refers to a byte-string value
        unsafe { raw::read_str_mib(self.0.as_ref()) }
    }
    fn write(&self, value: &'static [u8]) -> Result<()> {
        raw::write_str_mib(self.0.as_ref(), value)
    }
    fn update(&self, value: &'static [u8]) -> Result<&'static [u8]> {
        // this is safe because the only safe way to construct a `MibStr` is by
        // validating that the key refers to a byte-string value
        unsafe { raw::update_str_mib(self.0.as_ref(), value) }
    }
}

impl Access<&'static [u8]> for Name {
    fn read(&self) -> Result<&'static [u8]> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        // this is safe because the key refers to a byte string:
        unsafe { raw::read_str(&self.0) }
    }
    fn write(&self, value: &'static [u8]) -> Result<()> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        raw::write_str(&self.0, value)
    }
    fn update(&self, value: &'static [u8]) -> Result<&'static [u8]> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        // this is safe because the key refers to a byte string:
        unsafe { raw::update_str(&self.0, value) }
    }
}

impl<T: MibArg> Access<&'static str> for MibStr<T> {
    fn read(&self) -> Result<&'static str> {
        // this is safe because the only safe way to construct a `MibStr` is by
        // validating that the key refers to a byte-string value
        let s = unsafe { raw::read_str_mib(self.0.as_ref())? };
        Ok(str::from_utf8(s).unwrap())
    }
    fn write(&self, value: &'static str) -> Result<()> {
        raw::write_str_mib(self.0.as_ref(), value.as_bytes())
    }
    fn update(&self, value: &'static str) -> Result<&'static str> {
        // this is safe because the only safe way to construct a `MibStr` is by
        // validating that the key refers to a byte-string value
        let s =
            unsafe { raw::update_str_mib(self.0.as_ref(), value.as_bytes())? };
        Ok(str::from_utf8(s).unwrap())
    }
}

impl Access<&'static str> for Name {
    fn read(&self) -> Result<&'static str> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        // this is safe because the key refers to a byte string:
        let s = unsafe { raw::read_str(&self.0)? };
        Ok(str::from_utf8(s).unwrap())
    }
    fn write(&self, value: &'static str) -> Result<()> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        raw::write_str(&self.0, value.as_bytes())
    }
    fn update(&self, value: &'static str) -> Result<&'static str> {
        assert!(
            self.value_type_str(),
            "the name \"{:?}\" does not refer to a byte string",
            self
        );
        // this is safe because the key refers to a byte string:
        let s = unsafe { raw::update_str(&self.0, value.as_bytes())? };
        Ok(str::from_utf8(s).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::{Access, AsName, Mib, MibStr};
    #[test]
    fn bool_rw() {
        let name = b"thread.tcache.enabled\0".name();
        let tcache: bool = name.read().unwrap();

        let new_tcache = !tcache;

        name.write(new_tcache).unwrap();

        let mib: Mib<[usize; 3]> = name.mib().unwrap();
        let r: bool = mib.read().unwrap();
        assert_eq!(r, new_tcache);
    }

    #[test]
    fn u32_r() {
        let name = b"arenas.bin.0.nregs\0".name();
        let v: u32 = name.read().unwrap();

        let mib: Mib<[usize; 4]> = name.mib().unwrap();
        let r: u32 = mib.read().unwrap();
        assert_eq!(r, v);
    }

    #[test]
    fn size_t_r() {
        let name = b"arenas.lextent.0.size\0".name();
        let v: libc::size_t = name.read().unwrap();

        let mib: Mib<[usize; 4]> = name.mib().unwrap();
        let r: libc::size_t = mib.read().unwrap();
        assert_eq!(r, v);
    }

    #[test]
    fn ssize_t_rw() {
        let name = b"arenas.dirty_decay_ms\0".name();
        let v: libc::ssize_t = name.read().unwrap();
        name.write(v).unwrap();

        let mib: Mib<[usize; 2]> = name.mib().unwrap();
        let r: libc::ssize_t = mib.read().unwrap();
        assert_eq!(r, v);
    }

    #[test]
    fn u64_rw() {
        let name = b"epoch\0".name();
        let epoch: u64 = name.read().unwrap();
        name.write(epoch).unwrap();

        let mib: Mib<[usize; 1]> = name.mib().unwrap();
        let epoch: u64 = mib.read().unwrap();
        mib.write(epoch).unwrap();
    }

    #[test]
    fn str_rw() {
        let name = b"arena.0.dss\0".name();
        let dss: &'static [u8] = name.read().unwrap();
        name.write(dss).unwrap();

        let mib: MibStr<[usize; 3]> = name.mib_str().unwrap();
        let dss2: &'static [u8] = mib.read().unwrap();
        mib.write(dss2).unwrap();

        assert_eq!(dss, dss2);
    }
}

pub trait MibArg:
    Copy
    + Clone
    + PartialEq
    + Default
    + fmt::Debug
    + AsRef<[usize]>
    + AsMut<[usize]>
{
}
impl<T> MibArg for T where
    T: Copy
        + Clone
        + PartialEq
        + Default
        + fmt::Debug
        + AsRef<[usize]>
        + AsMut<[usize]>
{
}
