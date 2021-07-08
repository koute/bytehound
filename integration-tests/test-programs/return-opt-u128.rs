const CONSTANT: u128 = 0xaaaaaaaaaaaaaaaa5555555555555555;

extern {
    fn malloc( size: usize ) -> *const u8;
    fn abort() -> !;
}

#[inline(never)]
#[no_mangle]
fn func_1() -> Option< u128 > {
    unsafe { malloc( 123456 ); }
    Some( CONSTANT )
}

#[inline(never)]
#[no_mangle]
fn func_2() {
    if func_1() != Some( CONSTANT ) {
        unsafe { abort(); }
    }
}

fn main() {
    func_2();
}