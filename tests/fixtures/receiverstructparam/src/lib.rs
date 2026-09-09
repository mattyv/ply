pub struct Key {
    pub account: String,
    pub asset: String,
}

pub struct Sink {
    count: usize,
}

impl Sink {
    pub fn new() -> Self {
        Self { count: 0 }
    }

    pub fn ingest(&mut self, key: Key) -> usize {
        let _ = key;
        self.count += 1;
        self.count
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn period_us(&self) -> u64 {
        self.count as u64
    }
}

impl Default for Sink {
    fn default() -> Self {
        Self::new()
    }
}
