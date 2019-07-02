use std::io::{self, Read, Write};
use std::fs::File;
use std::fmt;
use std::mem;

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

fn stack_format< R, F, G >( format_callback: F, use_callback: G ) -> R
    where F: FnOnce( &mut &mut [u8] ),
          G: FnOnce( &[u8] ) -> R
{
    let mut buffer: [u8; 1024] = unsafe { mem::uninitialized() };
    let p = {
        let mut p = &mut buffer[..];
        format_callback( &mut p );
        p.as_ptr() as usize
    };

    let length = p - buffer.as_ptr() as usize;
    use_callback( &buffer[ 0..length ] )
}

#[test]
fn test_stack_format() {
    stack_format( |mut out| {
        let _ = write!( out, "foo = {}", "bar" );
        let _ = write!( out, ";" );
    }, |output| {
        assert_eq!( output, b"foo = bar;" );
    });
}

pub fn stack_format_bytes< R, F >( args: fmt::Arguments, callback: F ) -> R
    where F: FnOnce( &[u8] ) -> R
{
    stack_format( |out| {
        let _ = out.write_fmt( args );
    }, callback )
}

pub fn stack_null_terminate< R, F >( input: &[u8], callback: F ) -> R
    where F: FnOnce( &[u8] ) -> R
{
    stack_format( |out| {
        let _ = out.write_all( input );
        let _ = out.write_all( &[0] );
    }, callback )
}
