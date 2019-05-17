use std::time::Duration;
use std::mem;

use parking_lot::{
    Mutex,
    Condvar
};

pub struct Channel< T > {
    queue: Mutex< Vec< T > >,
    condvar: Condvar
}

impl< T > Channel< T > {
    pub fn new() -> Self {
        Channel {
            queue: Mutex::new( Vec::new() ),
            condvar: Condvar::new()
        }
    }

    #[allow(dead_code)]
    pub fn recv_all( &self, output: &mut Vec< T > ) {
        output.clear();
        let mut guard = self.queue.lock();
        if guard.is_empty() {
            self.condvar.wait( &mut guard );
        }

        mem::swap( &mut *guard, output );
    }

    pub fn timed_recv_all( &self, output: &mut Vec< T >, duration: Duration ) {
        output.clear();

        let mut guard = self.queue.lock();
        if guard.is_empty() {
            self.condvar.wait_for( &mut guard, duration );
        }

        mem::swap( &mut *guard, output );
    }

    pub fn send( &self, value: T ) -> usize {
        self.send_with( || value )
    }

    pub fn send_with< F: FnOnce() -> T >( &self, callback: F ) -> usize {
        let mut guard = self.queue.lock();
        guard.push( callback() );
        self.condvar.notify_all();

        guard.len()
    }

    #[allow(dead_code)]
    pub fn len( &self ) -> usize {
        self.queue.lock().len()
    }
}
