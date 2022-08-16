use std::time::Duration;
use std::mem;

use std::sync::{Mutex, Condvar};
use crate::utils::CacheAligned;

const CHUNK_SIZE: usize = 64;

#[repr(C)]
pub struct Channel< T > {
    queues: [CacheAligned< Mutex< Vec< T > > >; 5],
    condvar: CacheAligned< Condvar >
}

pub struct ChannelBuffer< T > {
    queues: [Vec< T >; 5]
}

impl< T > Default for ChannelBuffer< T > {
    fn default() -> Self {
        Self::new()
    }
}

impl< T > ChannelBuffer< T > {
    pub fn new() -> Self {
        ChannelBuffer {
            queues: Default::default()
        }
    }

    pub fn is_empty( &self ) -> bool {
        self.queues.iter().all( |queue| queue.is_empty() )
    }

    pub fn extend( &mut self, iter: impl IntoIterator< Item = T > ) {
        self.queues[0].extend( iter );
    }

    pub fn drain< 'a >( &'a mut self ) -> impl Iterator< Item = T > + 'a {
        self.queues.iter_mut().flat_map( |queue| queue.drain(..) )
    }
}

impl< T > Channel< T > {
    pub const fn new() -> Self {
        Channel {
            queues: [
                CacheAligned( Mutex::new( Vec::new() ) ),
                CacheAligned( Mutex::new( Vec::new() ) ),
                CacheAligned( Mutex::new( Vec::new() ) ),
                CacheAligned( Mutex::new( Vec::new() ) ),
                CacheAligned( Mutex::new( Vec::new() ) ),
            ],
            condvar: CacheAligned( Condvar::new() )
        }
    }

    pub fn timed_recv_all( &self, buffer: &mut ChannelBuffer< T >, duration: Duration ) {
        let mut found = false;
        for (queue, output) in self.queues.iter().zip( buffer.queues.iter_mut() ) {
            let mut guard = queue.lock().unwrap();
            if !guard.is_empty() {
                std::mem::swap( &mut *guard, output );
                found = true;
            }
        }

        if !found {
            let mut guard = self.queues[0].lock().unwrap();
            if guard.is_empty() {
                guard = self.condvar.wait_timeout( guard, duration ).unwrap().0;
            }
            mem::swap( &mut *guard, &mut buffer.queues[0] );
        }
    }

    pub fn send( &self, value: T ) -> usize {
        self.send_with( || value )
    }

    pub fn send_with< F: FnOnce() -> T >( &self, callback: F ) -> usize {
        let mut guard = self.queues[0].lock().unwrap();
        self.condvar.notify_all();
        guard.reserve( 1 );
        guard.push( callback() );
        guard.len()
    }

    pub fn chunked_send_with< F: FnOnce() -> T >( &self, callback: F ) -> usize {
        let mut guard = self.queues[0].lock().unwrap();
        let length = guard.len() + 1;
        if length % CHUNK_SIZE == 0 {
            self.condvar.notify_all();
        }

        guard.reserve( 1 );
        guard.push( callback() );
        length
    }

    pub fn sharded_chunked_send_with< F: FnOnce() -> T >( &self, key: usize, callback: F ) -> usize {
        let queue_index = key & 0b11 + 1;
        let mut guard = self.queues[ queue_index ].lock().unwrap();
        let length = guard.len() + 1;
        if length % CHUNK_SIZE == 0 {
            self.condvar.notify_all();
        }

        guard.reserve( 1 );
        guard.push( callback() );
        length
    }

    pub fn flush( &self ) {
        self.condvar.notify_all();
    }
}
