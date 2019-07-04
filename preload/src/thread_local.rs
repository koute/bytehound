macro_rules! thread_local_reentrant {
    (static $name:ident: $ty:ty = $ctor:expr;) => {
        struct $name;

        impl $name {
            fn pthread_key() -> libc::pthread_key_t {
                use std::sync::atomic::{AtomicUsize, Ordering};

                const INCOMPLETE: usize = 0;
                const COMPLETE: usize = 1;
                const RUNNING: usize = 2;

                static mut KEY: libc::pthread_key_t = 0;
                static ONCE: AtomicUsize = AtomicUsize::new( INCOMPLETE );

                if ONCE.load( Ordering::Acquire ) != COMPLETE {
                    loop {
                        let old = ONCE.compare_and_swap( INCOMPLETE, RUNNING, Ordering::SeqCst );
                        if old == INCOMPLETE {
                            unsafe {
                                libc::pthread_key_create( &mut KEY, Some( destructor ) );
                            }

                            ONCE.store( COMPLETE, Ordering::SeqCst );
                            break;
                        } else if old == COMPLETE {
                            break;
                        } else {
                            std::thread::yield_now();
                            continue;
                        }
                    }
                }

                unsafe extern fn destructor( cell: *mut libc::c_void ) {
                    let cell = cell as *mut (*mut $ty, bool);
                    let tls = std::ptr::replace( cell, (std::ptr::null_mut(), true) ).0;
                    if !tls.is_null() {
                        std::mem::drop( Box::from_raw( tls ) );
                    }
                }

                unsafe {
                    KEY
                }
            }

            fn new() -> $ty {
                $ctor
            }

            #[cold]
            #[inline(never)]
            unsafe fn construct( cell: *mut (*mut $ty, bool) ) -> *mut $ty {
                let was_already_initialized = (*cell).1;
                if was_already_initialized {
                    return std::ptr::null_mut();
                }

                (*cell).1 = true;

                let tls: $ty = Self::new();
                let tls = Box::into_raw( Box::new( tls ) );
                *cell = (tls, true);

                // Currently Rust triggers an allocation when registering
                // the TLS destructor, so we do it manually ourselves to avoid
                // an infinite loop.
                libc::pthread_setspecific( Self::pthread_key(), cell as *const libc::c_void );

                tls
            }

            #[inline]
            fn tls() -> &'static std::thread::LocalKey< std::cell::UnsafeCell< (*mut $ty, bool) > > {
                use std::cell::UnsafeCell;

                const EMPTY_TLS: UnsafeCell< (*mut $ty, bool) > = UnsafeCell::new( (0 as _, false) );
                thread_local! {
                    static TLS: UnsafeCell< (*mut $ty, bool) > = EMPTY_TLS;
                }

                &TLS
            }

            #[inline]
            pub unsafe fn get( &self ) -> Option< &'static mut $ty > {
                let tls = Self::tls();
                let mut ptr: *mut $ty = 0 as _;
                let _ = tls.try_with( |cell| {
                    let cell = cell.get();
                    ptr = (*cell).0;
                    if !(*cell).1 {
                        ptr = Self::construct( cell );
                    }
                });

                if ptr.is_null() {
                    return None;
                }

                Some( &mut *ptr )
            }
        }
    };
}

#[test]
fn test_thread_local_reentrant() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CTOR_COUNTER: AtomicUsize = AtomicUsize::new( 0 );
    static DTOR_COUNTER: AtomicUsize = AtomicUsize::new( 0 );

    struct Dummy {
        text: String
    }

    impl Drop for Dummy {
        fn drop( &mut self ) {
            DTOR_COUNTER.fetch_add( 1, Ordering::SeqCst );
        }
    }

    thread_local_reentrant! {
        static VALUE: Dummy = {
            let ctor_counter = CTOR_COUNTER.load( Ordering::SeqCst );
            let dtor_counter = DTOR_COUNTER.load( Ordering::SeqCst );

            assert!( unsafe { VALUE.get() }.is_none() );

            assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), ctor_counter );
            assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), dtor_counter );

            CTOR_COUNTER.fetch_add( 1, Ordering::SeqCst );
            Dummy {
                text: "foobar".into()
            }
        };
    }

    assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 0 );
    assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 0 );

    let thread = std::thread::spawn( || {
        assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 0 );
        assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 0 );

        let value = unsafe { VALUE.get() };
        assert_eq!( value.unwrap().text, "foobar" );

        assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 1 );
        assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 0 );
    });

    thread.join().unwrap();

    assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 1 );
    assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 1 );

    for _ in 0..2 {
        let value = unsafe { VALUE.get() };
        assert_eq!( value.unwrap().text, "foobar" );

        assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 2 );
        assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 1 );
    }
}
