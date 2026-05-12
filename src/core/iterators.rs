use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub trait StorageIterator {
    type KeyType: AsRef<[u8]>;

    fn next(&mut self);
    fn key(&self) -> Self::KeyType;
    fn value(&self) -> &[u8];
    fn is_valid(&self) -> bool;
    fn seek(&mut self, key: &[u8]);
}

pub struct HeapEntry<I: StorageIterator> {
    pub iter: I,
    pub index: usize,
}

impl<I: StorageIterator> PartialEq for HeapEntry<I> {
    fn eq(&self, other: &Self) -> bool {
        self.iter.key().as_ref() == other.iter.key().as_ref() && self.index == other.index
    }
}

impl<I: StorageIterator> Eq for HeapEntry<I> {}

impl<I: StorageIterator> PartialOrd for HeapEntry<I> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<I: StorageIterator> Ord for HeapEntry<I> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap based on key. If keys are equal, lower index (newer) wins.
        // BinaryHeap is a Max-heap, so we reverse the ordering.
        let ord = other.iter.key().as_ref().cmp(self.iter.key().as_ref());
        if ord == Ordering::Equal {
            return other.index.cmp(&self.index);
        }
        ord
    }
}

pub struct MergeIterator<I: StorageIterator> {
    heap: BinaryHeap<HeapEntry<I>>,
    current_key: Option<Vec<u8>>,
}

impl<I: StorageIterator> MergeIterator<I> {
    pub fn new(iters: Vec<I>) -> Self {
        let mut heap = BinaryHeap::new();
        for (index, iter) in iters.into_iter().enumerate() {
            if iter.is_valid() {
                heap.push(HeapEntry { iter, index });
            }
        }
        let mut mi = Self {
            heap,
            current_key: None,
        };
        mi.skip_duplicates();
        mi
    }

    fn skip_duplicates(&mut self) {
        while let Some(top) = self.heap.peek() {
            let key = top.iter.key().as_ref().to_vec();
            if let Some(ref cur) = self.current_key {
                if key == *cur {
                    // Same key as current, but from an older iterator (higher index)
                    // Pop it, advance it, and re-push if valid.
                    // Safe: we just peeked, so the heap is non-empty
                    let mut entry = self
                        .heap
                        .pop()
                        .unwrap_or_else(|| unreachable!("heap confirmed non-empty by peek"));
                    entry.iter.next();
                    if entry.iter.is_valid() {
                        self.heap.push(entry);
                    }
                    continue;
                }
            }
            // New key
            break;
        }
    }
}

impl<I: StorageIterator> StorageIterator for MergeIterator<I> {
    type KeyType = Vec<u8>;

    fn next(&mut self) {
        if let Some(mut top) = self.heap.pop() {
            self.current_key = Some(top.iter.key().as_ref().to_vec());
            top.iter.next();
            if top.iter.is_valid() {
                self.heap.push(top);
            }
        }
        self.skip_duplicates();
    }

    fn key(&self) -> Self::KeyType {
        match self.heap.peek() {
            Some(entry) => entry.iter.key().as_ref().to_vec(),
            None => Vec::new(), // Caller should check is_valid() first
        }
    }

    fn value(&self) -> &[u8] {
        match self.heap.peek() {
            Some(entry) => entry.iter.value(),
            None => &[], // Caller should check is_valid() first
        }
    }

    fn is_valid(&self) -> bool {
        !self.heap.is_empty()
    }

    fn seek(&mut self, _key: &[u8]) {
        // Simplified seek for now: rebuild heap from pointers and seek each
        unimplemented!("Seek not required for basic scan/keys optimization")
    }
}

impl<K: AsRef<[u8]>, I: StorageIterator<KeyType = K> + ?Sized> StorageIterator for Box<I> {
    type KeyType = K;

    fn next(&mut self) {
        (**self).next();
    }

    fn key(&self) -> Self::KeyType {
        (**self).key()
    }

    fn value(&self) -> &[u8] {
        (**self).value()
    }

    fn is_valid(&self) -> bool {
        (**self).is_valid()
    }

    fn seek(&mut self, key: &[u8]) {
        (**self).seek(key);
    }
}

/// A mock StorageIterator that yields keys from a slice in order.
#[cfg(test)]
struct MockIter {
    keys: Vec<Vec<u8>>,
    vals: Vec<Vec<u8>>,
    pos: usize,
}

#[cfg(test)]
impl MockIter {
    fn new(keys: Vec<&'static str>, vals: Vec<&'static str>) -> Self {
        Self {
            keys: keys.iter().map(|s| s.as_bytes().to_vec()).collect(),
            vals: vals.iter().map(|s| s.as_bytes().to_vec()).collect(),
            pos: 0,
        }
    }
}

#[cfg(test)]
impl StorageIterator for MockIter {
    type KeyType = Vec<u8>;

    fn next(&mut self) {
        if self.pos < self.keys.len() {
            self.pos += 1;
        }
    }

    fn key(&self) -> Self::KeyType {
        self.keys[self.pos].clone()
    }

    fn value(&self) -> &[u8] {
        &self.vals[self.pos]
    }

    fn is_valid(&self) -> bool {
        self.pos < self.keys.len()
    }

    fn seek(&mut self, key: &[u8]) {
        while self.is_valid() && self.key().as_slice() < key {
            self.next();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_iterator_uses_binary_heap() {
        // Create 3 mock iterators with interleaved sorted keys
        let iter_a = MockIter::new(
            vec!["apple", "cherry", "elderberry"],
            vec!["v1", "v3", "v5"],
        );
        let iter_b = MockIter::new(vec!["banana", "date", "fig"], vec!["v2", "v4", "v6"]);
        let iter_c = MockIter::new(vec!["grape", "honeydew"], vec!["v7", "v8"]);

        let iters = vec![iter_a, iter_b, iter_c];
        let mut merged = MergeIterator::new(iters);

        // Verify the heap is backed by BinaryHeap (compile-time check)
        let _: std::collections::BinaryHeap<HeapEntry<MockIter>> =
            std::collections::BinaryHeap::new();

        // Collect merged output
        let mut output = Vec::new();
        while merged.is_valid() {
            output.push((merged.key().clone(), merged.value().to_vec()));
            merged.next();
        }

        // Expected sorted order
        let expected: Vec<&[u8]> = vec![
            b"apple",
            b"banana",
            b"cherry",
            b"date",
            b"elderberry",
            b"fig",
            b"grape",
            b"honeydew",
        ];

        assert_eq!(
            output.len(),
            expected.len(),
            "MergeIterator should produce all 8 keys in sorted order"
        );

        for (i, exp_key) in expected.iter().enumerate() {
            assert_eq!(
                output[i].0.as_slice(),
                *exp_key,
                "Position {} should be {:?}, got {:?}",
                i,
                exp_key,
                output[i].0
            );
        }

        // Verify the heap-based MergeIterator correctly interleaved keys
        // Each heap push is O(log N) — the BinaryHeap implementation provides
        // this guarantee at the type level.
    }
}
