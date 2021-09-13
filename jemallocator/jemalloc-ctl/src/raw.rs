//! Raw `unsafe` access to the `malloctl` API.

use crate::error::{cvt, Result};
use crate::{mem, ptr, slice};
use libc::c_char;

/// Translates `name` to a `mib` (Management Information Base)
///
/// `mib`s are used to avoid repeated name lookups for applications that
/// repeatedly query the same portion of `jemalloc`s `mallctl` namespace.
///
/// On success, `mib` contains an array of integers. It is possible to pass
/// `mib` with a length smaller than the number of period-separated name
/// components. This results in a partial MIB that can be used as the basis for
/// constructing a complete MIB.
///
/// For name components that are integers (e.g. the `2` in `arenas.bin.2.size`),
/// the corresponding MIB component will always be that integer. Therefore, it
/// is legitimate to construct code like the following:
///
/// ```
/// #[global_allocator]
/// static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
///
/// fn main() {
///     use tikv_jemalloc_ctl::raw;
///     use libc::{c_uint, c_char};
///     unsafe {
///         let mut mib = [0; 4];
///         let nbins: c_uint = raw::read(b"arenas.nbins\0").unwrap();
///         raw::name_to_mib(b"arenas.bin.0.size\0", &mut mib).unwrap();
///         for i in 0..4 {
///             mib[2] = i;
///             let bin_size: usize = raw::read_mib(&mut mib).unwrap();
///             println!("arena bin {} has size {}", i, bin_size);
///         }
///     }
/// }
/// ```
pub fn name_to_mib(name: &[u8], mib: &mut [usize]) -> Result<()> {
    unsafe {
        validate_name(name);

        let mut len = mib.len();
        cvt(tikv_jemalloc_sys::mallctlnametomib(
            name as *const _ as *const c_char,
            mib.as_mut_ptr(),
            &mut len,
        ))?;
        assert_eq!(mib.len(), len);
        Ok(())
    }
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and reads its value.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=bool` for a key returning `u8`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn read_mib<T: Copy>(mib: &[usize]) -> Result<T> {
    let mut value = MaybeUninit { init: () };
    let mut len = mem::size_of::<T>();
    cvt(tikv_jemalloc_sys::mallctlbymib(
        mib.as_ptr(),
        mib.len(),
        &mut value.init as *mut _ as *mut _,
        &mut len,
        ptr::null_mut(),
        0,
    ))?;
    assert_eq!(len, mem::size_of::<T>());
    Ok(value.maybe_uninit)
}

/// Uses the null-terminated string `name` as key to the _MALLCTL NAMESPACE_ and
/// reads its value.
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=bool` for a key returning `u8`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn read<T: Copy>(name: &[u8]) -> Result<T> {
    validate_name(name);

    let mut value = MaybeUninit { init: () };
    let mut len = mem::size_of::<T>();
    cvt(tikv_jemalloc_sys::mallctl(
        name as *const _ as *const c_char,
        &mut value.init as *mut _ as *mut _,
        &mut len,
        ptr::null_mut(),
        0,
    ))?;
    assert_eq!(len, mem::size_of::<T>());
    Ok(value.maybe_uninit)
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and writes its `value`.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=u8` for a key expecting `bool`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn write_mib<T>(mib: &[usize], mut value: T) -> Result<()> {
    cvt(tikv_jemalloc_sys::mallctlbymib(
        mib.as_ptr(),
        mib.len(),
        ptr::null_mut(),
        ptr::null_mut(),
        &mut value as *mut _ as *mut _,
        mem::size_of::<T>(),
    ))
}

/// Uses the null-terminated string `name` as the key to the _MALLCTL NAMESPACE_
/// and writes it `value`
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=u8` for a key expecting `bool`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn write<T>(name: &[u8], mut value: T) -> Result<()> {
    validate_name(name);

    cvt(tikv_jemalloc_sys::mallctl(
        name as *const _ as *const c_char,
        ptr::null_mut(),
        ptr::null_mut(),
        &mut value as *mut _ as *mut _,
        mem::size_of::<T>(),
    ))
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and writes its `value`
/// returning its previous value.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=u8` for a key expecting `bool`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn update_mib<T>(mib: &[usize], mut value: T) -> Result<T> {
    let mut len = mem::size_of::<T>();
    cvt(tikv_jemalloc_sys::mallctlbymib(
        mib.as_ptr(),
        mib.len(),
        &mut value as *mut _ as *mut _,
        &mut len,
        &mut value as *mut _ as *mut _,
        len,
    ))?;
    assert_eq!(len, mem::size_of::<T>());
    Ok(value)
}

/// Uses the null-terminated string `name` as key to the _MALLCTL NAMESPACE_ and
/// writes its `value` returning its previous value.
///
/// # Safety
///
/// This function is `unsafe` because it is possible to use it to construct an
/// invalid `T`, for example, by passing `T=u8` for a key expecting `bool`. The
/// sizes of `bool` and `u8` match, but `bool` cannot represent all values that
/// `u8` can.
pub unsafe fn update<T>(name: &[u8], mut value: T) -> Result<T> {
    validate_name(name);

    let mut len = mem::size_of::<T>();
    cvt(tikv_jemalloc_sys::mallctl(
        name as *const _ as *const c_char,
        &mut value as *mut _ as *mut _,
        &mut len,
        &mut value as *mut _ as *mut _,
        len,
    ))?;
    assert_eq!(len, mem::size_of::<T>());
    Ok(value)
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and reads its value.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Safety
///
/// This function is unsafe because if the key does not return a pointer to a
/// null-terminated string the behavior is undefined.
///
/// For example, a key for a `u64` value can be used to read a pointer on 64-bit
/// platform, where this pointer will point to the address denoted by the `u64`s
/// representation. Also, a key to a `*mut extent_hooks_t` will return a pointer
/// that will not point to a null-terminated string.
///
/// This function needs to compute the length of the string by looking for the
/// null-terminator: `\0`. This requires reading the memory behind the pointer.
///
/// If the pointer is invalid (e.g. because it was converted from a `u64` that
/// does not represent a valid address), reading the string to look for `\0`
/// will dereference a non-dereferenceable pointer, which is undefined behavior.
///
/// If the pointer is valid but it does not point to a null-terminated string,
/// looking for `\0` will read garbage and might end up reading out-of-bounds,
/// which is undefined behavior.
pub unsafe fn read_str_mib(mib: &[usize]) -> Result<&'static [u8]> {
    let ptr: *const c_char = read_mib(mib)?;
    Ok(ptr2str(ptr))
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and writes its `value`.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Panics
///
/// If `value` is not a non-empty null-terminated string.
pub fn write_str_mib(mib: &[usize], value: &'static [u8]) -> Result<()> {
    assert!(!value.is_empty(), "value cannot be empty");
    assert_eq!(*value.last().unwrap(), b'\0');
    // This is safe because `value` will always point to a null-terminated
    // string, which makes it safe for all key value types: pointers to
    // null-terminated strings, pointers, pointer-sized integers, etc.
    unsafe { write_mib(mib, value.as_ptr() as *const c_char) }
}

/// Uses the MIB `mib` as key to the _MALLCTL NAMESPACE_ and writes its `value`
/// returning its previous value.
///
/// The [`name_to_mib`] API translates a string of the key (e.g. `arenas.nbins`)
/// to a `mib` (Management Information Base).
///
/// # Safety
///
/// This function is unsafe because if the key does not return a pointer to a
/// null-terminated string the behavior is undefined.
///
/// For example, a key for a `u64` value can be used to read a pointer on 64-bit
/// platform, where this pointer will point to the address denoted by the `u64`s
/// representation. Also, a key to a `*mut extent_hooks_t` will return a pointer
/// that will not point to a null-terminated string.
///
/// This function needs to compute the length of the string by looking for the
/// null-terminator: `\0`. This requires reading the memory behind the pointer.
///
/// If the pointer is invalid (e.g. because it was converted from a `u64` that
/// does not represent a valid address), reading the string to look for `\0`
/// will dereference a non-dereferenceable pointer, which is undefined behavior.
///
/// If the pointer is valid but it does not point to a null-terminated string,
/// looking for `\0` will read garbage and might end up reading out-of-bounds,
/// which is undefined behavior.
pub unsafe fn update_str_mib(
    mib: &[usize],
    value: &'static [u8],
) -> Result<&'static [u8]> {
    let ptr: *const c_char = update_mib(mib, value.as_ptr() as *const c_char)?;
    Ok(ptr2str(ptr))
}

/// Uses the null-terminated string `name` as key to the _MALLCTL NAMESPACE_ and
/// reads its value.
///
/// # Safety
///
/// This function is unsafe because if the key does not return a pointer to a
/// null-terminated string the behavior is undefined.
///
/// For example, a key for a `u64` value can be used to read a pointer on 64-bit
/// platform, where this pointer will point to the address denoted by the `u64`s
/// representation. Also, a key to a `*mut extent_hooks_t` will return a pointer
/// that will not point to a null-terminated string.
///
/// This function needs to compute the length of the string by looking for the
/// null-terminator: `\0`. This requires reading the memory behind the pointer.
///
/// If the pointer is invalid (e.g. because it was converted from a `u64` that
/// does not represent a valid address), reading the string to look for `\0`
/// will dereference a non-dereferenceable pointer, which is undefined behavior.
///
/// If the pointer is valid but it does not point to a null-terminated string,
/// looking for `\0` will read garbage and might end up reading out-of-bounds,
/// which is undefined behavior.
pub unsafe fn read_str(name: &[u8]) -> Result<&'static [u8]> {
    let ptr: *const c_char = read(name)?;
    Ok(ptr2str(ptr))
}

/// Uses the null-terminated string `name` as key to the _MALLCTL NAMESPACE_ and
/// writes its `value`.
pub fn write_str(name: &[u8], value: &'static [u8]) -> Result<()> {
    assert!(!value.is_empty(), "value cannot be empty");
    assert_eq!(*value.last().unwrap(), b'\0');
    // This is safe because `value` will always point to a null-terminated
    // string, which makes it safe for all key value types: pointers to
    // null-terminated strings, pointers, pointer-sized integers, etc.
    unsafe { write(name, value.as_ptr() as *const c_char) }
}

/// Uses the null-terminated string `name` as key to the _MALLCTL NAMESPACE_ and
/// writes its `value` returning its previous value.
///
/// # Safety
///
/// This function is unsafe because if the key does not return a pointer to a
/// null-terminated string the behavior is undefined.
///
/// For example, a key for a `u64` value can be used to read a pointer on 64-bit
/// platform, where this pointer will point to the address denoted by the `u64`s
/// representation. Also, a key to a `*mut extent_hooks_t` will return a pointer
/// that will not point to a null-terminated string.
///
/// This function needs to compute the length of the string by looking for the
/// null-terminator: `\0`. This requires reading the memory behind the pointer.
///
/// If the pointer is invalid (e.g. because it was converted from a `u64` that
/// does not represent a valid address), reading the string to look for `\0`
/// will dereference a non-dereferenceable pointer, which is undefined behavior.
///
/// If the pointer is valid but it does not point to a null-terminated string,
/// looking for `\0` will read garbage and might end up reading out-of-bounds,
/// which is undefined behavior.
pub unsafe fn update_str(
    name: &[u8],
    value: &'static [u8],
) -> Result<&'static [u8]> {
    let ptr: *const c_char = update(name, value.as_ptr() as *const c_char)?;
    Ok(ptr2str(ptr))
}

/// Converts a non-empty null-terminated character string at `ptr` into a valid
/// null-terminated UTF-8 string.
///
/// # Panics
///
/// If `ptr.is_null()`.
///
/// # Safety
///
/// If `ptr` does not point to a null-terminated character string the behavior
/// is undefined.
unsafe fn ptr2str(ptr: *const c_char) -> &'static [u8] {
    assert!(
        !ptr.is_null(),
        "attempt to convert a null-ptr to a UTF-8 string"
    );
    let len = libc::strlen(ptr);
    slice::from_raw_parts(ptr as *const u8, len + 1)
}

fn validate_name(name: &[u8]) {
    assert!(!name.is_empty(), "empty byte string");
    assert_eq!(
        *name.last().unwrap(),
        b'\0',
        "non-null terminated byte string"
    );
}

union MaybeUninit<T: Copy> {
    init: (),
    maybe_uninit: T,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(not(target_arch = "mips64el"))] // FIXME: SIGFPE
    fn test_ptr2str() {
        unsafe {
            //{ // This is undefined behavior:
            //    let cstr = b"";
            //    let rstr = ptr2str(cstr as *const _ as *const c_char);
            //    assert!(rstr.is_err());
            // }
            {
                let cstr = b"\0";
                let rstr = ptr2str(cstr as *const _ as *const c_char);
                assert_eq!(rstr.len(), 1);
                assert_eq!(rstr, b"\0");
            }
            {
                let cstr = b"foo  baaar\0";
                let rstr = ptr2str(cstr as *const _ as *const c_char);
                assert_eq!(rstr.len(), b"foo  baaar\0".len());
                assert_eq!(rstr, b"foo  baaar\0");
            }
        }
    }
}
