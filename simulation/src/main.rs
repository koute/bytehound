use std::time::{Duration, Instant};
use std::mem::forget;
use oorandom::Rand64;

#[cfg(feature = "jemallocator")]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[inline(never)]
fn allocate_temporary(
    rng: &mut Rand64,
    list: &mut Vec< Vec< u8 > >
) {
    if rng.rand_float() >= 0.25 {
        let mut payload = Vec::new();
        payload.resize( rng.rand_range( 1..2048 ) as usize, 127 );
        list.push( payload );
    }

    if rng.rand_float() >= 0.25 && !list.is_empty() {
        let index = rng.rand_range( 0..list.len() as u64 );
        list.swap_remove( index as usize );
    }
}

#[inline(never)]
fn allocate_linear_leak_never_deallocated(
    rng: &mut Rand64
) {
    if rng.rand_float() >= 0.99 {
        let mut payload = Vec::new();
        payload.resize( rng.rand_range( 1..128 ) as usize, 127 );
        forget( payload );
    }
}

#[inline(never)]
fn allocate_linear_leak_deallocated_at_the_end(
    rng: &mut Rand64,
    list: &mut Vec< Vec< u8 > >
) {
    if rng.rand_float() >= 0.99 {
        let mut payload = Vec::new();
        payload.resize( rng.rand_range( 1..128 ) as usize, 127 );
        list.push( payload );
    }
}

#[derive(Default)]
struct StateBoundedLeak {
    list: Vec< Vec< u8 > >,
    total_size: usize,
    max_total_size: Option< usize >
}

impl Drop for StateBoundedLeak {
    fn drop( &mut self ) {
        self.list.drain( .. ).for_each( |item| {
            forget( item );
        });
    }
}

#[inline(never)]
fn allocate_bounded_leak(
    rng: &mut Rand64,
    state: &mut StateBoundedLeak,
    should_stop_leaking: bool
) {
    if rng.rand_float() >= 0.99 {
        let mut payload = Vec::new();
        payload.resize( rng.rand_range( 1..128 ) as usize, 127 );
        state.total_size += payload.len();
        state.list.push( payload );

        if should_stop_leaking && state.max_total_size.is_none() {
            state.max_total_size = Some( state.total_size );
        }

        if let Some( max_total_size ) = state.max_total_size {
            while state.total_size > max_total_size {
                let index = rng.rand_range( 0..state.list.len() as u64 );
                let chunk = state.list.swap_remove( index as usize );
                state.total_size -= chunk.len();
            }
        }
    }
}

#[inline(never)]
fn allocate_both_temporary_and_linear_leak(
    rng: &mut Rand64,
    list: &mut Vec< Vec< u8 > >
) {
    let mut payload = Vec::new();
    payload.resize( rng.rand_range( 1..128 ) as usize, 127 );

    if rng.rand_float() >= 0.999 {
        forget( payload );
    } else {
        if rng.rand_float() >= 0.25 {
            list.push( payload );
        }
        if rng.rand_float() >= 0.25 {
            list.pop();
        }
    }
}

const RUNTIME: u64 = 10;

fn main() {
    let mut rng = Rand64::new( 12341234 );
    let start = Instant::now();
    let mut state_temporary = Vec::new();
    let mut state_linear_leak = Vec::new();
    let mut state_both_temporary_and_linear_leak = Vec::new();
    let mut state_bounded_leak = StateBoundedLeak::default();

    while start.elapsed() < Duration::from_secs( RUNTIME ) {
        allocate_temporary( &mut rng, &mut state_temporary );
        allocate_linear_leak_never_deallocated( &mut rng );
        allocate_linear_leak_deallocated_at_the_end(
            &mut rng,
            &mut state_linear_leak
        );
        allocate_bounded_leak(
            &mut rng,
            &mut state_bounded_leak,
            start.elapsed() >= Duration::from_secs( RUNTIME / 2 )
        );
        allocate_both_temporary_and_linear_leak(
            &mut rng,
            &mut state_both_temporary_and_linear_leak
        );

        if rng.rand_float() >= 0.5 {
            std::thread::sleep(
                Duration::from_micros( rng.rand_range( 1..10 ) )
            );
        }
    }
}
