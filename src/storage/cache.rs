use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

type BlockId = (u64, usize);

#[derive(Clone, Debug)]
pub struct GlobalBlockCache {
    cache: Arc<Mutex<LruCache<BlockId, Vec<u8>>>>,
}

impl GlobalBlockCache {
    pub fn new(size_mb: usize, block_size: usize) -> Arc<Self> {
        let max_blocks = (size_mb * 1024 * 1024) / block_size;
        let capacity = NonZeroUsize::new(max_blocks.max(1)).unwrap();

        Arc::new(Self {
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
        })
    }

    pub fn get(&self, table_id: u64, block_idx: usize) -> Option<Vec<u8>> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(&(table_id, block_idx)).cloned()
    }

    pub fn put(&self, table_id: u64, block_idx: usize, data: Vec<u8>) {
        let mut cache = self.cache.lock().unwrap();
        cache.put((table_id, block_idx), data);
    }

    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.lock().unwrap();
        CacheStats {
            len: cache.len(),
            cap: cache.cap().get(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub len: usize,
    pub cap: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic_ops() {
        let cache = GlobalBlockCache::new(1, 4096);

        cache.put(1, 0, vec![1, 2, 3]);
        assert_eq!(cache.get(1, 0), Some(vec![1, 2, 3]));

        assert_eq!(cache.get(1, 1), None);

        cache.put(1, 1, vec![4, 5, 6]);
        assert_eq!(cache.get(1, 1), Some(vec![4, 5, 6]));
    }

    #[test]
    fn test_cache_eviction() {
        let cache = GlobalBlockCache::new(1, 16384);
        let max_entries = (1024 * 1024) / 16384;

        for i in 0..max_entries + 10 {
            cache.put(1, i, vec![i as u8]);
        }

        assert!(cache.get(1, 0).is_none());
        assert!(cache.get(1, 1).is_none());

        let recent_idx = max_entries + 5;
        assert_eq!(cache.get(1, recent_idx), Some(vec![recent_idx as u8]));
    }

    #[test]
    fn test_cache_different_tables() {
        let cache = GlobalBlockCache::new(1, 4096);

        cache.put(1, 0, vec![1]);
        cache.put(2, 0, vec![2]);
        cache.put(3, 0, vec![3]);

        assert_eq!(cache.get(1, 0), Some(vec![1]));
        assert_eq!(cache.get(2, 0), Some(vec![2]));
        assert_eq!(cache.get(3, 0), Some(vec![3]));

        assert_eq!(cache.get(1, 1), None);
        assert_eq!(cache.get(2, 1), None);
    }

    #[test]
    fn test_cache_overwrite() {
        let cache = GlobalBlockCache::new(1, 4096);

        cache.put(1, 0, vec![1, 2, 3]);
        cache.put(1, 0, vec![4, 5, 6]);

        assert_eq!(cache.get(1, 0), Some(vec![4, 5, 6]));
    }

    #[test]
    fn test_cache_stats() {
        let cache = GlobalBlockCache::new(1, 4096);

        let stats = cache.stats();
        assert_eq!(stats.len, 0);
        assert_eq!(stats.cap, (1024 * 1024) / 4096);

        cache.put(1, 0, vec![1]);
        let stats = cache.stats();
        assert_eq!(stats.len, 1);

        cache.put(1, 1, vec![2]);
        let stats = cache.stats();
        assert_eq!(stats.len, 2);
    }

    #[test]
    fn test_cache_minimum_capacity() {
        let cache = GlobalBlockCache::new(0, 4096);
        let stats = cache.stats();
        assert_eq!(stats.cap, 1);

        cache.put(1, 0, vec![1]);
        assert_eq!(cache.get(1, 0), Some(vec![1]));

        cache.put(1, 1, vec![2]);
        assert_eq!(cache.get(1, 1), Some(vec![2]));
        assert_eq!(cache.get(1, 0), None);
    }

    #[test]
    fn test_cache_large_blocks() {
        let cache = GlobalBlockCache::new(1, 1024 * 1024);
        let stats = cache.stats();
        assert_eq!(stats.cap, 1);

        let large_block = vec![0u8; 1024 * 1024];
        cache.put(1, 0, large_block.clone());
        assert_eq!(cache.get(1, 0), Some(large_block));
    }

    #[test]
    fn test_cache_concurrent_access() {
        use std::thread;

        let cache = GlobalBlockCache::new(1, 4096);
        let cache_clone = Arc::clone(&cache.cache.lock().unwrap().cap().get().into());

        let handles: Vec<_> = (0..10)
            .map(|i| {
                let cache = cache.clone();
                thread::spawn(move || {
                    cache.put(1, i, vec![i as u8]);
                    cache.get(1, i)
                })
            })
            .collect();

        for handle in handles {
            let result = handle.join().unwrap();
            assert!(result.is_some());
        }
    }
}
