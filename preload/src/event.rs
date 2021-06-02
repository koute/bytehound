use std::time::Duration;
use std::sync::Arc;
use std::num::NonZeroUsize;

use common::Timestamp;

use crate::channel::Channel;
use crate::global::WeakThreadHandle;
use crate::unwind::Backtrace;

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(C)]
pub struct InternalAllocationId {
    pub thread: u64,
    pub allocation: u64,
    pub checksum: u64
}

impl std::fmt::Display for InternalAllocationId {
    fn fmt( &self, fmt: &mut std::fmt::Formatter ) -> std::fmt::Result {
        if self.is_valid() {
            write!( fmt, "{{{}, {}}}", self.thread, self.allocation )
        } else {
            write!( fmt, "{{0x{:X}, 0x{:X}, 0x{:X}}}", self.thread, self.allocation, self.checksum )
        }
    }
}

// These are just arbitrarily picked to be big and random enough.
const UNTRACKED_THREAD: u64 = 0xEAD1F4ED4A816337;
const UNTRACKED_ALLOCATION: u64 = 0xEBBDDB5F42D04E74;

const CHECKSUM_CONSTANT: u64 = 0x8000000000000000;

impl InternalAllocationId {
    pub const UNTRACKED: Self = Self::new( UNTRACKED_THREAD, UNTRACKED_ALLOCATION );

    pub const fn new( thread: u64, allocation: u64 ) -> Self {
        InternalAllocationId {
            thread,
            allocation,
            checksum: thread ^ allocation ^ CHECKSUM_CONSTANT
        }
    }

    pub fn is_untracked( self ) -> bool {
        self == Self::UNTRACKED
    }

    pub fn is_valid( self ) -> bool {
        self.thread ^ self.allocation ^ CHECKSUM_CONSTANT == self.checksum
    }
}

impl From< InternalAllocationId > for common::event::AllocationId {
    fn from( id: InternalAllocationId ) -> Self {
        if id.is_untracked() {
            common::event::AllocationId::UNTRACKED
        } else if !id.is_valid() {
            common::event::AllocationId::INVALID
        } else {
            common::event::AllocationId {
                thread: id.thread,
                allocation: id.allocation
            }
        }
    }
}

pub(crate) enum InternalEvent {
    Alloc {
        id: InternalAllocationId,
        address: NonZeroUsize,
        size: usize,
        usable_size: usize,
        preceding_free_space: usize,
        flags: u32,
        backtrace: Backtrace,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    Realloc {
        id: InternalAllocationId,
        old_address: NonZeroUsize,
        new_address: NonZeroUsize,
        new_size: usize,
        new_usable_size: usize,
        new_preceding_free_space: usize,
        new_flags: u32,
        backtrace: Backtrace,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    Free {
        id: InternalAllocationId,
        address: NonZeroUsize,
        backtrace: Backtrace,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    Exit,
    GrabMemoryDump,
    SetMarker {
        value: u32
    },
    Mmap {
        pointer: usize,
        requested_address: usize,
        length: usize,
        mmap_protection: u32,
        mmap_flags: u32,
        offset: u64,
        backtrace: Backtrace,
        file_descriptor: u32,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    Munmap {
        ptr: usize,
        len: usize,
        backtrace: Backtrace,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    Mallopt {
        param: i32,
        value: i32,
        result: i32,
        backtrace: Backtrace,
        timestamp: Timestamp,
        thread: WeakThreadHandle
    },
    OverrideNextTimestamp {
        timestamp: Timestamp
    },
    AddressSpaceUpdated {
        maps: String,
        new_binaries: Vec< Arc< nwind::BinaryData > >
    }
}

static EVENT_CHANNEL: Channel< InternalEvent > = Channel::new();

pub(crate) fn send_event( event: InternalEvent ) {
    EVENT_CHANNEL.send( event );
}

#[inline(always)]
pub(crate) fn send_event_throttled< F: FnOnce() -> InternalEvent >( callback: F ) {
    EVENT_CHANNEL.chunked_send_with( 64, callback );
}

pub(crate) fn timed_recv_all_events( output: &mut Vec< InternalEvent >, duration: Duration ) {
    EVENT_CHANNEL.timed_recv_all( output, duration )
}

pub(crate) fn flush() {
    EVENT_CHANNEL.flush();
}
