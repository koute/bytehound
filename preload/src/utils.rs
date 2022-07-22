use std::io::{self, Read, Write};
use std::fs::File;
use std::fmt;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::ffi::OsStr;
use std::path::Path;
use std::os::unix::ffi::OsStrExt;

use crate::{EXECUTABLE, PID};
use crate::syscall;

pub fn read_file( path: &str ) -> io::Result< Vec< u8 > > {
    let mut fp = File::open( path )?;
    let mut buffer = Vec::new();
    fp.read_to_end( &mut buffer )?;
    Ok( buffer )
}

pub fn copy< I: Read, O: Write >( mut input: I, mut output: O ) -> io::Result< () > {
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read( &mut buffer )?;
        if count == 0 {
            break;
        }
        output.write_all( &buffer[ 0..count ] )?;
    }
    Ok(())
}

pub struct RestoreFileCreationMaskOnDrop( libc::c_int );
impl Drop for RestoreFileCreationMaskOnDrop {
    fn drop( &mut self ) {
        syscall::umask( self.0 );
    }
}

pub fn temporarily_change_umask( umask: libc::c_int ) -> RestoreFileCreationMaskOnDrop {
    let old_umask = syscall::umask( umask );
    RestoreFileCreationMaskOnDrop( old_umask )
}

const STACK_BUFFER_LEN: usize = 1024;

pub struct Buffer {
    buffer: [MaybeUninit< u8 >; STACK_BUFFER_LEN],
    length: usize
}

impl std::fmt::Debug for Buffer {
    fn fmt( &self, formatter: &mut std::fmt::Formatter ) -> std::fmt::Result {
        let slice = self.as_slice();
        if let Ok( string ) = std::str::from_utf8( slice ) {
            formatter.write_str( string )
        } else {
            self.buffer[ 0..self.length ].fmt( formatter )
        }
    }
}

impl Buffer {
    pub const fn new() -> Self {
        unsafe {
            Self {
                buffer: MaybeUninit::< [MaybeUninit< u8 >; STACK_BUFFER_LEN] >::uninit().assume_init(),
                length: 0
            }
        }
    }

    pub const fn from_fixed_slice< const N: usize >( slice: &[u8; N] ) -> Self {
        let mut buffer = Self::new();
        let mut nth = 0;
        while nth < N {
            buffer.buffer[ nth ] = MaybeUninit::new( slice[ nth ] );
            nth += 1;
        }
        buffer.length = N;
        buffer
    }

    pub fn from_slice( slice: &[u8] ) -> Option< Self > {
        if slice.len() > STACK_BUFFER_LEN {
            return None;
        }

        let mut buffer = Self::new();
        buffer.write( slice ).unwrap();
        Some( buffer )
    }

    pub fn to_str( &self ) -> Option< &str > {
        std::str::from_utf8( self.as_slice() ).ok()
    }

    pub fn as_slice( &self ) -> &[u8] {
        unsafe { std::slice::from_raw_parts( self.buffer.as_ptr() as *const u8, self.length ) }
    }

    fn as_slice_mut( &mut self ) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut( self.buffer.as_mut_ptr() as *mut u8, self.length ) }
    }

    fn push( &mut self, byte: u8 ) {
        if self.length >= self.buffer.len() {
            return;
        }

        self.buffer[ self.length ] = MaybeUninit::new( byte );
        self.length += 1;
    }
}

impl std::ops::Deref for Buffer {
    type Target = [u8];
    fn deref( &self ) -> &Self::Target {
        self.as_slice()
    }
}

impl AsRef< OsStr > for Buffer {
    fn as_ref( &self ) -> &OsStr {
        OsStr::from_bytes( self.as_slice() )
    }
}

impl AsRef< Path > for Buffer {
    fn as_ref( &self ) -> &Path {
        Path::new( self )
    }
}

impl Write for Buffer {
    fn write( &mut self, input: &[u8] ) -> io::Result< usize > {
        let count = std::cmp::min( input.len(), STACK_BUFFER_LEN - self.length );
        unsafe {
            std::ptr::copy_nonoverlapping( input.as_ptr(), self.buffer[ self.length.. ].as_mut_ptr() as *mut u8, count );
        }
        self.length += count;
        Ok( count )
    }

    fn flush( &mut self ) -> io::Result< () > {
        Ok(())
    }
}

fn stack_format< R, F, G >( format_callback: F, use_callback: G ) -> R
    where F: FnOnce( &mut Buffer ),
          G: FnOnce( &mut [u8] ) -> R
{
    let mut buffer = Buffer::new();
    format_callback( &mut buffer );
    use_callback( buffer.as_slice_mut() )
}

#[test]
fn test_stack_format_short() {
    stack_format( |out| {
        write!( out, "foo = {}", "bar" ).unwrap();
        write!( out, ";" ).unwrap();
    }, |output| {
        assert_eq!( output, b"foo = bar;" );
    });
}

#[test]
fn test_stack_format_long() {
    stack_format( |out| {
        for _ in 0..STACK_BUFFER_LEN {
            write!( out, "X" ).unwrap();
        }
        assert!( write!( out, "Y" ).is_err() );
    }, |output| {
        assert_eq!( output.len(), STACK_BUFFER_LEN );
        assert!( output.iter().all( |&byte| byte == b'X' ) );
    });
}

pub fn stack_format_bytes< R, F >( args: fmt::Arguments, callback: F ) -> R
    where F: FnOnce( &mut [u8] ) -> R
{
    stack_format( |out| {
        let _ = out.write_fmt( args );
    }, callback )
}

pub fn stack_null_terminate< R, F >( input: &[u8], callback: F ) -> R
    where F: FnOnce( &mut [u8] ) -> R
{
    stack_format( |out| {
        let _ = out.write_all( input );
        let _ = out.write_all( &[0] );
    }, callback )
}

pub fn generate_filename( pattern: &[u8], counter: Option< &AtomicUsize > ) -> Buffer {
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
                    output.push( ch );
                },
                b'p' => {
                    let pid = *PID;
                    write!( &mut output, "{}", pid ).unwrap();
                },
                b't' => {
                    let timestamp = unsafe { libc::time( ptr::null_mut() ) };
                    write!( &mut output, "{}", timestamp ).unwrap();
                },
                b'e' => {
                    let executable = String::from_utf8_lossy( &*EXECUTABLE );
                    let executable = &executable[ executable.rfind( "/" ).map( |index| index + 1 ).unwrap_or( 0 ).. ];
                    write!( &mut output, "{}", executable ).unwrap();
                },
                b'n' => {
                    if let Some( counter ) = counter {
                        let value = counter.fetch_add( 1, Ordering::SeqCst );
                        write!( &mut output, "{}", value ).unwrap();
                    }
                },
                _ => {}
            }
        } else {
            output.push( ch );
        }
    }

    output
}
