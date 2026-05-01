use crate::engine::Engine;
use crate::error::Result;
use crate::value::Value;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Eq, PartialEq)]
struct HeapItem {
    idx: usize,
    key: Vec<u8>,
    value: Value,
}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min‑heap behaviour.
        other.key.cmp(&self.key)
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Wrapper around multiple RocksDB iterators that merges them in key order.
pub struct MergeIterator {
    iters: Vec<Box<dyn IteratorItem>>,
    heap: BinaryHeap<HeapItem>,
    limit: Option<usize>,
    yielded: usize,
}

trait IteratorItem {
    fn seek(&mut self, key: &[u8]);
    fn current(&self) -> Option<(&[u8], &Value)>;
    fn next(&mut self) -> Option<(&[u8], &Value)>;
}

impl MergeIterator {
    pub async fn new(
        _engine: &Engine,
        _cf: Option<&str>,
        _start: Option<Vec<u8>>,
        _end: Option<Vec<u8>>,
        _limit: Option<usize>,
    ) -> Result<Self> {
        // Construction logic omitted for brevity.
        // In a real implementation you would create the underlying RocksDB iterators
        // based on the supplied parameters.
        unimplemented!()
    }

    /// Seek all underlying iterators to `key` and rebuild the heap.
    fn seek(&mut self, key: &[u8]) {
        // Seek each child iterator.  If a child returns `None` it is exhausted and
        // will simply be omitted from the heap.
        for it in &mut self.iters {
            it.seek(key);
        }

        // Clear the heap and push the new heads.
        self.heap.clear();
        for (idx, it) in self.iters.iter_mut().enumerate() {
            if let Some((k, v)) = it.current() {
                self.heap.push(HeapItem {
                    idx,
                    key: k.to_vec(),
                    value: v.clone(),
                });
            }
        }
    }

    /// Return the next key/value pair from the merged view.
    /// Returns owned `Vec<u8>` and `Value` so that the lifetime does not depend on the iterator.
    pub fn next(&mut self) -> Option<(Vec<u8>, Value)> {
        if let Some(limit) = self.limit {
            if self.yielded >= limit {
                return None;
            }
        }

        let top = self.heap.pop()?;
        let idx = top.idx;
        let key = top.key.clone();
        let value = top.value.clone();

        // Advance the iterator that supplied the top element.
        if let Some((k, v)) = self.iters[idx].next() {
            self.heap.push(HeapItem {
                idx,
                key: k.to_vec(),
                value: v.clone(),
            });
        }

        self.yielded += 1;
        Some((key, value))
    }
}
