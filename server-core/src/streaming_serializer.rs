use serde::{Serialize, Serializer};
use std::cell::RefCell;

pub struct StreamingSerializer<F, R, T>
where
    F: FnOnce() -> R,
    R: Iterator<Item = T>,
{
    callback: RefCell<Option<F>>,
}

impl<F, R, T> StreamingSerializer<F, R, T>
where
    F: FnOnce() -> R,
    R: Iterator<Item = T>,
    T: Serialize,
{
    pub fn new(callback: F) -> Self {
        StreamingSerializer {
            callback: RefCell::new(Some(callback)),
        }
    }
}

impl<F, R, T> Serialize for StreamingSerializer<F, R, T>
where
    F: FnOnce() -> R,
    R: Iterator<Item = T>,
    T: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;

        let iter = (self.callback.borrow_mut().take().unwrap())();
        let mut seq = serializer.serialize_seq(None)?;
        for element in iter {
            seq.serialize_element(&element)?;
        }

        seq.end()
    }
}
