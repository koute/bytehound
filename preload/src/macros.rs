#[inline(never)]
#[cold]
pub fn fatal_error(args: std::fmt::Arguments) -> ! {
    use log::Log;

    let record = log::RecordBuilder::new()
        .level(log::Level::Error)
        .args(args)
        .build();

    unsafe {
        crate::init::SYSCALL_LOGGER.log(&record);
        crate::init::FILE_LOGGER.log(&record);
    }

    panic!();
}

macro_rules! panic {
    () => {{
        $crate::macros::fatal_error( format_args!(
            "panic triggered at {}:{}",
            std::file!(), std::line!()
        ));
    }};

    ($($token:expr),+) => {{
        $crate::macros::fatal_error( format_args!(
            "panic triggered at {}:{}: {}",
            std::file!(), std::line!(), format!( $($token),+ )
        ));
    }};
}

macro_rules! assert_eq {
    ($lhs: expr, $rhs: expr) => {{
        match (&$lhs, &$rhs) {
            (lhs, rhs) => {
                if lhs != rhs {
                    $crate::macros::fatal_error(format_args!(
                        "assertion failed at {}:{}: {} == {}",
                        std::file!(),
                        std::line!(),
                        stringify!($lhs),
                        stringify!($rhs)
                    ));
                }
            }
        }
    }};
}

macro_rules! assert_ne {
    ($lhs: expr, $rhs: expr) => {{
        match (&$lhs, &$rhs) {
            (lhs, rhs) => {
                if lhs == rhs {
                    $crate::macros::fatal_error(format_args!(
                        "assertion failed at {}:{}: {} != {}",
                        std::file!(),
                        std::line!(),
                        stringify!($lhs),
                        stringify!($rhs)
                    ));
                }
            }
        }
    }};
}
