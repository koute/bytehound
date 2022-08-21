use std::alloc::Layout;
use std::ptr::write_volatile;

pub unsafe fn alloc( size: usize ) -> *mut u8 {
    let pointer = std::alloc::alloc( Layout::from_size_align( size, 1 ).unwrap() );
    write_volatile( pointer, 1 );
    pointer
}

unsafe fn alloc_zeroed( size: usize ) -> *mut u8 {
    let pointer = std::alloc::alloc_zeroed( Layout::from_size_align( size, 1 ).unwrap() );
    write_volatile( pointer, 1 );
    pointer
}

unsafe fn realloc( pointer: *mut u8, old_size: usize, new_size: usize ) -> *mut u8 {
    let pointer = std::alloc::realloc( pointer, Layout::from_size_align( old_size, 1 ).unwrap(), new_size );
    write_volatile( pointer, 1 );
    pointer
}

unsafe fn free( pointer: *mut u8, size: usize ) {
    std::alloc::dealloc( pointer, Layout::from_size_align( size, 1 ).unwrap() )
}

#[inline(never)]
pub unsafe fn run_test() {
    assert_eq!( CONSTRUCTOR_RAN, true );

    alloc( 10 );
    let a1 = alloc( 100 );
    let a2 = alloc( 1000 );
    realloc( a2, 1000, 10000 );
    alloc_zeroed( 100000 );

    free( a1, 100 );

    let mut a5 = libc::malloc( 200 ) as *mut u8;
    write_volatile( a5, 1 );
    a5 = libc::realloc( a5 as _, 400 ) as *mut u8;
    write_volatile( a5, 1 );
    libc::free( a5 as _ );
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Elf_auxv_t {
    a_type: usize,
    a_val: *const (),
}

const AT_NULL: u32 = 0;
static mut CONSTRUCTOR_RAN: bool = false;

#[used]
#[link_section = ".init_array.00099"]
static INIT_ARRAY: unsafe extern "C" fn( libc::c_int, *mut *mut u8, *mut *mut u8 ) = {
    unsafe extern "C" fn function( _argc: libc::c_int, _argv: *mut *mut u8, mut envp: *mut *mut u8 ) {
        while !(*envp).is_null() {
            envp = envp.add( 1 );
        }

        let mut auxp: *const Elf_auxv_t = envp.add(1).cast();
        let mut count = 0;
        loop {
            let Elf_auxv_t { a_type, .. } = *auxp;
            match a_type as _ {
                AT_NULL => break,
                _ => (),
            }
            auxp = auxp.add(1);
            count += 1;
        }

        assert_ne!( count, 0 );
        CONSTRUCTOR_RAN = true;
    }
    function
};
