#[derive(Default)]
pub struct NoHash;

impl std::hash::BuildHasher for NoHash {
    type Hasher = NoHasher;
    fn build_hasher(&self) -> Self::Hasher {
        NoHasher(0)
    }
}

pub struct NoHasher(u64);
impl std::hash::Hasher for NoHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, _bytes: &[u8]) {
        unimplemented!()
    }

    fn write_u32(&mut self, value: u32) {
        self.0 ^= value as u64;
    }

    fn write_u64(&mut self, value: u64) {
        self.0 ^= value;
    }

    fn write_usize(&mut self, value: usize) {
        self.0 ^= value as u64;
    }
}
