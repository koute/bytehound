const CONSTANT: f64 = -3.7206620809969885e-103;

extern {
    fn malloc( size: usize ) -> *const u8;
    fn abort() -> !;
}

#[inline(never)]
#[no_mangle]
fn func_1() -> f64 {
    unsafe { malloc( 123456 ); }
    CONSTANT
}

#[inline(never)]
#[no_mangle]
fn func_2() {
    if func_1() != CONSTANT {
        unsafe { abort(); }
    }
}

fn main() {
    func_2();
}