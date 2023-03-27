use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::syscall;
use crate::{EXECUTABLE, PID};

pub fn read_file(path: &str) -> io::Result<Vec<u8>> {
    let mut fp = File::open(path)?;
    let mut buffer = Vec::new();
    fp.read_to_end(&mut buffer)?;
    Ok(buffer)
}

pub fn copy<I: Read, O: Write>(mut input: I, mut output: O) -> io::Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        output.write_all(&buffer[0..count])?;
    }
    Ok(())
}

pub struct RestoreFileCreationMaskOnDrop(libc::c_int);
impl Drop for RestoreFileCreationMaskOnDrop {
    fn drop(&mut self) {
        syscall::umask(self.0);
    }
}

pub fn temporarily_change_umask(umask: libc::c_int) -> RestoreFileCreationMaskOnDrop {
    let old_umask = syscall::umask(umask);
    RestoreFileCreationMaskOnDrop(old_umask)
}

const STACK_BUFFER_DEFAULT_LEN: usize = 1024;

pub struct Buffer<const L: usize = STACK_BUFFER_DEFAULT_LEN> {
    buffer: [MaybeUninit<u8>; L],
    length: usize,
}

impl<const L: usize> std::fmt::Debug for Buffer<L> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        let slice = self.as_slice();
        if let Ok(string) = std::str::from_utf8(slice) {
            formatter.write_str(string)
        } else {
            self.buffer[0..self.length].fmt(formatter)
        }
    }
}

impl<const L: usize> Buffer<L> {
    pub const fn new() -> Self {
        unsafe {
            Self {
                buffer: MaybeUninit::<[MaybeUninit<u8>; L]>::uninit().assume_init(),
                length: 0,
            }
        }
    }

    pub const fn from_fixed_slice<const N: usize>(slice: &[u8; N]) -> Self {
        let mut buffer = Self::new();
        let mut nth = 0;
        while nth < N {
            buffer.buffer[nth] = MaybeUninit::new(slice[nth]);
            nth += 1;
        }
        buffer.length = N;
        buffer
    }

    pub fn from_slice(slice: &[u8]) -> Option<Self> {
        if slice.len() > L {
            return None;
        }

        let mut buffer = Self::new();
        buffer.write(slice).unwrap();
        Some(buffer)
    }

    pub fn to_str(&self) -> Option<&str> {
        std::str::from_utf8(self.as_slice()).ok()
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.buffer.as_ptr() as *const u8, self.length) }
    }

    fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.buffer.as_mut_ptr() as *mut u8, self.length) }
    }

    fn push(&mut self, byte: u8) {
        if self.length >= self.buffer.len() {
            return;
        }

        self.buffer[self.length] = MaybeUninit::new(byte);
        self.length += 1;
    }
}

impl<const L: usize> std::ops::Deref for Buffer<L> {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<const L: usize> AsRef<OsStr> for Buffer<L> {
    fn as_ref(&self) -> &OsStr {
        OsStr::from_bytes(self.as_slice())
    }
}

impl<const L: usize> AsRef<Path> for Buffer<L> {
    fn as_ref(&self) -> &Path {
        Path::new(self)
    }
}

impl<const L: usize> Write for Buffer<L> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let count = std::cmp::min(input.len(), L - self.length);
        unsafe {
            std::ptr::copy_nonoverlapping(
                input.as_ptr(),
                self.buffer[self.length..].as_mut_ptr() as *mut u8,
                count,
            );
        }
        self.length += count;
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn stack_format<const L: usize, R, F, G>(format_callback: F, use_callback: G) -> R
where
    F: FnOnce(&mut Buffer<L>),
    G: FnOnce(&mut [u8]) -> R,
{
    let mut buffer = Buffer::new();
    format_callback(&mut buffer);
    use_callback(buffer.as_slice_mut())
}

#[test]
fn test_stack_format_short() {
    stack_format(
        |out: &mut Buffer<STACK_BUFFER_DEFAULT_LEN>| {
            write!(out, "foo = {}", "bar").unwrap();
            write!(out, ";").unwrap();
        },
        |output| {
            assert_eq!(output, b"foo = bar;");
        },
    );
}

#[test]
fn test_stack_format_long() {
    stack_format(
        |out: &mut Buffer<STACK_BUFFER_DEFAULT_LEN>| {
            for _ in 0..STACK_BUFFER_DEFAULT_LEN {
                write!(out, "X").unwrap();
            }
            assert!(write!(out, "Y").is_err());
        },
        |output| {
            assert_eq!(output.len(), STACK_BUFFER_DEFAULT_LEN);
            assert!(output.iter().all(|&byte| byte == b'X'));
        },
    );
}

pub fn stack_format_bytes<R, F>(args: fmt::Arguments, callback: F) -> R
where
    F: FnOnce(&mut [u8]) -> R,
{
    stack_format(
        |out: &mut Buffer<STACK_BUFFER_DEFAULT_LEN>| {
            let _ = out.write_fmt(args);
        },
        callback,
    )
}

pub fn stack_null_terminate<R, F>(input: &[u8], callback: F) -> R
where
    F: FnOnce(&mut [u8]) -> R,
{
    stack_format(
        |out: &mut Buffer<STACK_BUFFER_DEFAULT_LEN>| {
            let _ = out.write_all(input);
            let _ = out.write_all(&[0]);
        },
        callback,
    )
}

pub fn generate_filename(
    pattern: &[u8],
    counter: Option<&AtomicUsize>,
) -> Buffer<STACK_BUFFER_DEFAULT_LEN> {
    let mut output = Buffer::new();
    let mut seen_percent = false;
    for &ch in pattern.as_ref() {
        if !seen_percent && ch == b'%' {
            seen_percent = true;
            continue;
        }

        if seen_percent {
            seen_percent = false;
            match ch {
                b'%' => {
                    output.push(ch);
                }
                b'p' => {
                    let pid = *PID;
                    write!(&mut output, "{}", pid).unwrap();
                }
                b't' => {
                    let timestamp = unsafe { libc::time(ptr::null_mut()) };
                    write!(&mut output, "{}", timestamp).unwrap();
                }
                b'e' => {
                    let executable = String::from_utf8_lossy(&*EXECUTABLE);
                    let executable =
                        &executable[executable.rfind("/").map(|index| index + 1).unwrap_or(0)..];
                    write!(&mut output, "{}", executable).unwrap();
                }
                b'n' => {
                    if let Some(counter) = counter {
                        let value = counter.fetch_add(1, Ordering::SeqCst);
                        write!(&mut output, "{}", value).unwrap();
                    }
                }
                _ => {}
            }
        } else {
            output.push(ch);
        }
    }

    output
}

#[repr(align(64))]
pub struct CacheAligned<T>(pub T);

impl<T> std::ops::Deref for CacheAligned<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for CacheAligned<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub type HashMap<K, V> = hashbrown::HashMap<K, V, ahash::random_state::RandomState>;
pub type HashSet<T> = hashbrown::HashSet<T, ahash::random_state::RandomState>;
pub type Entry<'a, K, V> = hashbrown::hash_map::Entry<'a, K, V, ahash::random_state::RandomState>;
pub const fn empty_hashmap<K, V>() -> HashMap<K, V> {
    hashbrown::HashMap::with_hasher(ahash::random_state::RandomState::with_seeds(
        0x40b1d1a46e72d5af,
        0xfa484741bb13cbac,
        0x059a48d9d09ed59d,
        0x08ab62f8e225add9,
    ))
}
