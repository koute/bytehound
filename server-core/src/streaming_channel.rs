use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll};

use futures;
use futures::task::AtomicWaker;

struct Inner<T> {
    buffer: VecDeque<T>,
    waker: AtomicWaker,
    sender_closed: bool,
    receiver_closed: bool,
}

pub struct Sender<T>(Arc<(Condvar, Mutex<Inner<T>>)>);

impl<T> Sender<T> {
    pub fn send(&mut self, value: T) -> Result<(), ()> {
        let mut inner = (self.0).1.lock().unwrap();
        if inner.receiver_closed {
            inner.buffer.clear();
            return Err(());
        }

        while inner.buffer.len() >= 16 {
            inner = (self.0).0.wait(inner).unwrap();
            if inner.receiver_closed {
                inner.buffer.clear();
                return Err(());
            }
        }

        inner.buffer.push_back(value);

        inner.waker.wake();

        Ok(())
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let mut inner = (self.0).1.lock().unwrap();
        inner.sender_closed = true;
        inner.waker.wake();
    }
}

pub struct Receiver<T>(Arc<(Condvar, Mutex<Inner<T>>)>);
impl<T> futures_core::stream::Stream for Receiver<T> {
    type Item = Result<T, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut inner = (self.0).1.lock().unwrap();
        match inner.buffer.pop_front() {
            Some(value) => {
                (self.0).0.notify_all();
                Poll::Ready(Some(Ok(value)))
            }
            None => {
                if inner.sender_closed {
                    return Poll::Ready(None);
                }
                inner.waker.register(cx.waker());
                Poll::Pending
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let mut inner = (self.0).1.lock().unwrap();
        inner.receiver_closed = true;
        (self.0).0.notify_all();
    }
}

pub fn streaming_channel<T>() -> (Sender<T>, Receiver<T>) {
    let inner = Inner {
        buffer: VecDeque::new(),
        waker: AtomicWaker::new(),
        sender_closed: false,
        receiver_closed: false,
    };

    let condvar = Condvar::new();
    let inner = Arc::new((condvar, Mutex::new(inner)));
    let tx = Sender(inner.clone());
    let rx = Receiver(inner.clone());

    (tx, rx)
}
