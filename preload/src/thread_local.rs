use {
    std::{
        cell::{
            UnsafeCell
        },
        mem::{
            ManuallyDrop
        },
        ops::{
            Deref,
            DerefMut
        },
        sync::{
            atomic::{
                AtomicUsize,
                Ordering
            }
        }
    }
};

#[repr(transparent)]
#[must_use]
pub struct TlsValue< T >( T );

impl< T > Deref for TlsValue< T > {
    type Target = T;
    fn deref( &self ) -> &Self::Target {
        &self.0
    }
}

impl< T > DerefMut for TlsValue< T > {
    fn deref_mut( &mut self ) -> &mut Self::Target {
        &mut self.0
    }
}

enum SlotKind< T > {
    Uninitialized,
    Initialized( ManuallyDrop< T > ),
    Destroyed,
    Initializing
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TlsAccessError {
    Uninitialized,
    Destroyed
}

pub const INCOMPLETE: usize = 0;
const COMPLETE: usize = 1;
const RUNNING: usize = 2;

#[cold]
pub unsafe fn initialize_pthread_key< T >(
    key: *mut libc::pthread_key_t,
    once: &AtomicUsize
) -> libc::pthread_key_t {
    unsafe extern fn destructor< T >( cell: *mut libc::c_void ) {
        let kind = cell as *mut SlotKind< T >;
        if let SlotKind::Initialized( mut value ) = std::ptr::replace( kind, SlotKind::Destroyed ) {
            std::mem::ManuallyDrop::drop( &mut value );
        }
    }

    if once.load( Ordering::Acquire ) != COMPLETE {
        loop {
            let old = once.compare_exchange( INCOMPLETE, RUNNING, Ordering::SeqCst, Ordering::SeqCst );
            if old.is_ok() {
                libc::pthread_key_create( key, Some( destructor::< T > ) );
                once.store( COMPLETE, Ordering::SeqCst );
                break;
            } else if old == Err( COMPLETE ) {
                break;
            } else {
                std::thread::yield_now();
                continue;
            }
        }
    }

    *key
}

#[cold]
#[inline(never)]
unsafe fn initialize_slot< K >( slot: &Slot< K::Value > ) -> &K::Value
    where K: Key + ?Sized
{
    let pointer: *mut SlotKind< K::Value > = slot.0.get();

    assert!( matches!( *pointer, SlotKind::Uninitialized ) );
    *pointer = SlotKind::Initializing;

    let value = K::construct( move |value| {
        libc::pthread_setspecific( K::initialize_pthread_key(), pointer as *const libc::c_void );
        TlsValue( value )
    });

    *pointer = SlotKind::Initialized( ManuallyDrop::new( value.0 ) );

    match *pointer {
        SlotKind::Initialized( ref value ) => &value,
        _ => unreachable!()
    }
}

pub trait Key {
    type Value;

    fn construct( callback: impl FnOnce( Self::Value ) -> TlsValue< Self::Value > ) -> TlsValue< Self::Value >;
    fn initialize_pthread_key() -> libc::pthread_key_t;
    fn access_raw< R >( callback: impl FnOnce( &Slot< Self::Value > ) -> R ) -> R;

    #[inline(always)]
    fn access< R >( callback: impl FnOnce( &Self::Value ) -> R ) -> Result< R, TlsAccessError > {
        Self::access_raw( |slot| {
            let value = match *unsafe { &mut *slot.0.get() } {
                SlotKind::Initialized( ref value ) => value,
                SlotKind::Destroyed => return Err( TlsAccessError::Destroyed ),
                SlotKind::Initializing => return Err( TlsAccessError::Uninitialized ),
                SlotKind::Uninitialized => unsafe { initialize_slot::< Self >( slot ) }
            };

            Ok( callback( value ) )
        })
    }
}

macro_rules! thread_local_reentrant {
    (static $name:ident: $ty:ty = |$callback:ident| $ctor:expr;) => {
        struct $name;
        impl $crate::thread_local::Key for $name {
            type Value = $ty;

            fn construct(
                callback: impl FnOnce( Self::Value ) -> $crate::thread_local::TlsValue< Self::Value >
            ) -> $crate::thread_local::TlsValue< Self::Value > {
                let $callback = callback;
                $ctor
            }

            fn initialize_pthread_key() -> libc::pthread_key_t {
                static mut KEY: libc::pthread_key_t = 0;
                static ONCE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new( $crate::thread_local::INCOMPLETE );

                unsafe {
                    $crate::thread_local::initialize_pthread_key::< $ty >( std::ptr::addr_of_mut!( KEY ), &ONCE )
                }
            }

            #[inline(always)]
            fn access_raw< R >( callback: impl FnOnce( &$crate::thread_local::Slot< Self::Value > ) -> R ) -> R {
                thread_local! {
                    static TLS: $crate::thread_local::Slot< $ty > = const { $crate::thread_local::Slot::uninit() };
                }
                TLS.with( |slot| callback( slot ) )
            }
        }

        impl $name {
            #[allow(dead_code)]
            #[inline(always)]
            pub fn with< R >( &self, callback: impl FnOnce( &$ty ) -> R ) -> Option< R > {
                <Self as $crate::thread_local::Key>::access( callback ).ok()
            }

            #[allow(dead_code)]
            #[inline(always)]
            pub fn with_ex< R >( &self, callback: impl FnOnce( &$ty ) -> R ) -> Result< R, $crate::thread_local::TlsAccessError > {
                <Self as $crate::thread_local::Key>::access( callback )
            }
        }
    };

    (static $name:ident: $ty:ty = $ctor:expr;) => {
        thread_local_reentrant! {
            static $name: $ty = |callback| {
                callback( (|| { $ctor })() )
            };
        }
    };
}

pub struct Slot< T >( UnsafeCell< SlotKind< T > > );

impl< T > Slot< T > {
    #[inline]
    pub const fn uninit() -> Self {
        Slot( UnsafeCell::new( SlotKind::Uninitialized ) )
    }
}

#[test]
fn test_slot_does_not_need_drop() {
    assert_eq!( std::mem::needs_drop::< Slot< String > >(), false );
}

#[test]
fn test_thread_local_reentrant_basic() {
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

            assert!( VALUE.with( |_| () ).is_none() );

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

        let value = VALUE.with( |value| value.text.clone() );
        assert_eq!( value.unwrap(), "foobar" );

        assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 1 );
        assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 0 );
    });

    thread.join().unwrap();

    assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 1 );
    assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 1 );

    for _ in 0..2 {
        let value = VALUE.with( |value| value.text.clone() );
        assert_eq!( value.unwrap(), "foobar" );

        assert_eq!( CTOR_COUNTER.load( Ordering::SeqCst ), 2 );
        assert_eq!( DTOR_COUNTER.load( Ordering::SeqCst ), 1 );
    }
}

#[test]
fn test_thread_local_reentrant_custom_constructor() {
    struct Tls {
        value: u32
    }

    thread_local_reentrant! {
        static TLS: Tls = |callback| {
            let tls = Tls {
                value: 10
            };

            let mut tls = callback( tls );
            assert_eq!( tls.value, 10 );
            tls.value = 20;
            tls
        };
    }

    let value = TLS.with( |value| value.value );
    assert_eq!( value.unwrap(), 20 );
}

#[test]
fn test_thread_local_reentrant_uninitialized() {
    struct Tls {
        value: u32
    }

    thread_local_reentrant! {
        static TLS: Tls = |callback| {
            assert_eq!( TLS.with_ex( |value| value.value ), Err( TlsAccessError::Uninitialized ) );
            let mut tls = callback( Tls { value: 10 } );
            assert_eq!( tls.value, 10 );
            assert_eq!( TLS.with_ex( |value| value.value ), Err( TlsAccessError::Uninitialized ) );
            tls.value = 20;
            tls
        };
    }

    let value = TLS.with( |value| value.value );
    assert_eq!( value.unwrap(), 20 );
}
