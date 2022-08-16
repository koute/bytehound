use std::time::Duration;
use std::mem;

use std::sync::{Mutex, Condvar};

pub struct Channel< T > {
    queue: Mutex< Vec< T > >,
    condvar: Condvar
}

impl< T > Channel< T > {
    pub const fn new() -> Self {
        Channel {
            queue: Mutex::new( Vec::new() ),
            condvar: Condvar::new()
        }
    }

    #[allow(dead_code)]
    pub fn recv_all( &self, output: &mut Vec< T > ) {
        output.clear();
        let mut guard = self.queue.lock().unwrap();
        if guard.is_empty() {
            guard = self.condvar.wait( guard ).unwrap();
        }

        mem::swap( &mut *guard, output );
    }

    pub fn timed_recv_all( &self, output: &mut Vec< T >, duration: Duration ) {
        output.clear();

        let mut guard = self.queue.lock().unwrap();
        if guard.is_empty() {
            guard = self.condvar.wait_timeout( guard, duration ).unwrap().0;
        }

        mem::swap( &mut *guard, output );
    }

    pub fn send( &self, value: T ) -> usize {
        self.send_with( || value )
    }

    pub fn send_with< F: FnOnce() -> T >( &self, callback: F ) -> usize {
        let mut guard = self.queue.lock().unwrap();
        self.condvar.notify_all();
        guard.reserve( 1 );
        guard.push( callback() );
        guard.len()
    }

    pub fn chunked_send_with< F: FnOnce() -> T >( &self, chunk_size: usize, callback: F ) -> usize {
        let mut guard = self.queue.lock().unwrap();
        let length = guard.len() + 1;
        if length % chunk_size == 0 {
            self.condvar.notify_all();
        }

        guard.reserve( 1 );
        guard.push( callback() );
        length
    }

    pub fn flush( &self ) {
        self.condvar.notify_all();
    }

    #[allow(dead_code)]
    pub fn len( &self ) -> usize {
        self.queue.lock().unwrap().len()
    }
}
