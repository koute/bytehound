use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;

use parking_lot::{Mutex, RwLock};

use common::Timestamp;
use common::event::AllocationId;

use crate::global::StrongThreadHandle;
use crate::unwind::Backtrace;
use crate::event::{InternalAllocation, InternalAllocationId, InternalEvent};

pub struct BufferedAllocation {
    pub timestamp: Timestamp,
    pub allocation: InternalAllocation,
    pub backtrace: Backtrace
}

pub struct AllocationBucket {
    pub id: AllocationId,
    pub events: smallvec::SmallVec< [BufferedAllocation; 1] >
}

impl AllocationBucket {
    fn is_long_lived( &self, now: Timestamp ) -> bool {
        now.as_usecs() >= self.events[0].timestamp.as_usecs() + crate::opt::get().temporary_allocation_lifetime_threshold * 1000
    }
}

#[derive(Default)]
struct AllocationTrackerRegistry {
    per_thread: HashMap< u64, AllocationTracker, crate::nohash::NoHash >,
    global: Mutex< crate::ordered_map::OrderedMap< (u64, u64), AllocationBucket, crate::nohash::NoHash > >
}

static ENABLED: AtomicBool = AtomicBool::new( false );

lazy_static! {
    static ref ALLOCATION_TRACKER_REGISTRY: RwLock< AllocationTrackerRegistry > = Default::default();
}

#[derive(Default)]
struct AllocationTrackerState {
    buckets: crate::ordered_map::OrderedMap< u64, AllocationBucket, crate::nohash::NoHash >,
}

fn get_shard_key( id: AllocationId ) -> usize {
    id.thread as usize
}

#[repr(transparent)]
pub struct AllocationTracker( Arc< Mutex< AllocationTrackerState > > );

pub fn initialize() {
    ENABLED.store( crate::opt::get().cull_temporary_allocations, Ordering::SeqCst );
    let _ = ALLOCATION_TRACKER_REGISTRY.write();
}

pub fn on_thread_created( unique_tid: u64 ) -> AllocationTracker {
    let tracker = AllocationTracker( Arc::new( Mutex::new( AllocationTrackerState::default() ) ) );

    ALLOCATION_TRACKER_REGISTRY.write().per_thread.insert( unique_tid, AllocationTracker( tracker.0.clone() ) );
    tracker
}

pub fn on_thread_destroyed( unique_tid: u64 ) {
    let mut registry = ALLOCATION_TRACKER_REGISTRY.write();
    if let Some( local_tracker ) = registry.per_thread.remove( &unique_tid ) {
        let timestamp = crate::timestamp::get_timestamp();
        let mut local_tracker = local_tracker.0.lock();
        flush_pending( &mut local_tracker.buckets, timestamp );

        let mut global_tracker = registry.global.lock();
        flush_pending( &mut global_tracker, timestamp );

        while let Some( (allocation_id, bucket) ) = local_tracker.buckets.pop_front() {
            global_tracker.insert( (unique_tid, allocation_id), bucket );
        }
    }
}

pub fn on_tick() {
    let registry = ALLOCATION_TRACKER_REGISTRY.read();
    let timestamp = crate::timestamp::get_timestamp();
    {
        let mut global_tracker = registry.global.lock();
        flush_pending( &mut global_tracker, timestamp );
    }
    for tracker in registry.per_thread.values() {
        let mut local_tracker = tracker.0.lock();
        flush_pending( &mut local_tracker.buckets, timestamp );
    }
}

pub fn on_exit() {
    ENABLED.store( false, Ordering::SeqCst );

    let mut buckets = Vec::new();
    let mut registry = ALLOCATION_TRACKER_REGISTRY.write();
    for (_, tracker) in std::mem::take( &mut registry.per_thread ) {
        let mut tracker = tracker.0.lock();
        while let Some( (_, bucket) ) = tracker.buckets.pop_front() {
            buckets.push( bucket );
        }
    }

    {
        let mut global = registry.global.lock();
        while let Some( (_, bucket) ) = global.pop_front() {
            buckets.push( bucket );
        }
    }

    info!( "Flushing {} bucket(s) on exit", buckets.len() );
    for bucket in buckets {
        crate::event::send_event_throttled_sharded( get_shard_key( bucket.id ), move || {
            InternalEvent::AllocationBucket( bucket )
        });
    }
}

fn flush_pending< K >(
    allocations: &mut crate::ordered_map::OrderedMap< K, AllocationBucket, crate::nohash::NoHash >,
    timestamp: Timestamp
) where K: Copy + PartialEq + Eq + std::hash::Hash {
    let temporary_allocation_pending_threshold = crate::opt::get().temporary_allocation_pending_threshold.unwrap_or( !0 );
    while let Some( key ) = allocations.front_key() {
        let should_flush =
            allocations.len() > temporary_allocation_pending_threshold ||
            allocations.get( &key ).unwrap().is_long_lived( timestamp );

        if should_flush {
            let bucket = allocations.remove( &key ).unwrap();
            crate::event::send_event_throttled_sharded( get_shard_key( bucket.id ), move || {
                InternalEvent::AllocationBucket( bucket )
            });
        } else {
            break;
        }
    }
}

pub fn on_allocation(
    id: InternalAllocationId,
    allocation: InternalAllocation,
    backtrace: Backtrace,
    thread: StrongThreadHandle
) {
    let timestamp = crate::timestamp::get_timestamp();
    let id: AllocationId = id.into();
    debug_assert_eq!( id.thread, thread.unique_tid() );

    if thread.is_dead() {
        let mut zombie_events = thread.zombie_events().lock();
        zombie_events.push(
            InternalEvent::Alloc {
                id,
                timestamp,
                allocation,
                backtrace,
            }
        );

        return;
    } else if ENABLED.load( Ordering::Relaxed ) && !id.is_untracked() {
        let mut bucket = AllocationBucket {
            id,
            events: Default::default()
        };

        let address = allocation.address;
        bucket.events.push( BufferedAllocation { timestamp, allocation, backtrace } );

        let mut tracker = thread.allocation_tracker().0.lock();
        if tracker.buckets.insert( id.allocation, bucket ).is_some() {
            error!( "Duplicate allocation 0x{:08X} with ID {}; this should never happen", address, id );
        }

        flush_pending( &mut tracker.buckets, timestamp );
        return;
    }

    crate::event::send_event_throttled_sharded( get_shard_key( id ), move || {
        InternalEvent::Alloc {
            id,
            timestamp,
            allocation,
            backtrace,
        }
    });

    std::mem::drop( thread );
}

pub fn on_reallocation(
    id: InternalAllocationId,
    old_address: NonZeroUsize,
    allocation: InternalAllocation,
    backtrace: Backtrace,
    thread: StrongThreadHandle
) {
    let timestamp = crate::timestamp::get_timestamp();
    let id: AllocationId = id.into();
    if id.is_invalid() {
        // TODO: If we're culling temporary allocations try to find one with the same address and flush it.
        error!( "Allocation 0x{:08X} with invalid ID {} was reallocated; this should never happen; you probably have an out-of-bounds write somewhere", old_address, id );
    } if thread.is_dead() {
        let mut zombie_events = thread.zombie_events().lock();
        zombie_events.push(
            InternalEvent::Realloc {
                id,
                timestamp,
                old_address,
                allocation,
                backtrace,
            }
        );

        return;
    } else if !thread.is_dead() && ENABLED.load( Ordering::Relaxed ) && !id.is_untracked() {
        fn emit(
            bucket: &mut AllocationBucket,
            timestamp: Timestamp,
            id: AllocationId,
            old_address: NonZeroUsize,
            allocation: InternalAllocation,
            backtrace: Backtrace
        ) {
            if bucket.events.last().unwrap().allocation.address != old_address {
                error!(
                    "Reallocation with ID {} has old pointer 0x{:016X} while it should have 0x{:016X}; this should never happen",
                    id,
                    old_address,
                    allocation.address
                );
            }

            bucket.events.push( BufferedAllocation { timestamp, allocation, backtrace } );
        }

        if id.thread == thread.unique_tid() {
            let mut tracker = thread.allocation_tracker().0.lock();
            flush_pending( &mut tracker.buckets, timestamp );

            if let Some( bucket ) = tracker.buckets.get_mut( &id.allocation ) {
                return emit( bucket, timestamp, id, old_address, allocation, backtrace );
            }
        } else {
            let registry = ALLOCATION_TRACKER_REGISTRY.read();
            if let Some( registry ) = registry.per_thread.get( &id.thread ) {
                let mut tracker = registry.0.lock();
                flush_pending( &mut tracker.buckets, timestamp );

                if let Some( bucket ) = tracker.buckets.get_mut( &id.allocation ) {
                    return emit( bucket, timestamp, id, old_address, allocation, backtrace );
                }
            } else {
                let mut buckets = registry.global.lock();
                flush_pending( &mut buckets, timestamp );

                if let Some( bucket ) = buckets.get_mut( &(id.thread, id.allocation) ) {
                    return emit( bucket, timestamp, id, old_address, allocation, backtrace );
                }
            }
        }
    }

    crate::event::send_event_throttled_sharded( get_shard_key( id ), move || {
        InternalEvent::Realloc {
            id,
            timestamp,
            old_address,
            allocation,
            backtrace,
        }
    });

    std::mem::drop( thread );
}

pub fn on_free(
    id: InternalAllocationId,
    address: NonZeroUsize,
    backtrace: Option< Backtrace >,
    thread: StrongThreadHandle
) {
    let timestamp = crate::timestamp::get_timestamp();
    let id: AllocationId = id.into();
    if id.is_invalid() {
        // TODO: If we're culling temporary allocations try to find one with the same address and flush it.
        error!( "Allocation 0x{:08X} with invalid ID {} was freed; this should never happen; you probably have an out-of-bounds write somewhere", address.get(), id );
    } if thread.is_dead() {
        let mut zombie_events = thread.zombie_events().lock();
        zombie_events.push(
            InternalEvent::Free {
                timestamp,
                id,
                address,
                backtrace,
                tid: thread.system_tid()
            }
        );

        return;
    } else if !thread.is_dead() && ENABLED.load( Ordering::Relaxed ) && !id.is_untracked() && !id.is_invalid() {
        let bucket =
            if id.thread == thread.unique_tid() {
                let mut tracker = thread.allocation_tracker().0.lock();
                flush_pending( &mut tracker.buckets, timestamp );

                tracker.buckets.remove( &id.allocation )
            } else {
                let registry = ALLOCATION_TRACKER_REGISTRY.read();
                if let Some( registry ) = registry.per_thread.get( &id.thread ) {
                    let mut tracker = registry.0.lock();
                    flush_pending( &mut tracker.buckets, timestamp );

                    tracker.buckets.remove( &id.allocation )
                } else {
                    let mut allocations = registry.global.lock();
                    flush_pending( &mut allocations, timestamp );

                    allocations.remove( &(id.thread, id.allocation) )
                }
            };

        if let Some( bucket ) = bucket {
            if bucket.is_long_lived( timestamp ) {
                crate::event::send_event_throttled_sharded( get_shard_key( bucket.id ), move || {
                    InternalEvent::AllocationBucket( bucket )
                });
            } else {
                return;
            }
        }
    }

    crate::event::send_event_throttled_sharded( get_shard_key( id ), move || {
        InternalEvent::Free {
            timestamp,
            id,
            address,
            backtrace,
            tid: thread.system_tid()
        }
    });
}
