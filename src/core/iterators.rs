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
                    let mut entry = self.heap.pop().unwrap();
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
        self.heap.peek().unwrap().iter.key().as_ref().to_vec()
    }

    fn value(&self) -> &[u8] {
        self.heap.peek().unwrap().iter.value()
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
