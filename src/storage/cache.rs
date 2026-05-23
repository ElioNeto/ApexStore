use lru::LruCache;
use parking_lot::Mutex;
use std::num::NonZeroUsize;
use std::sync::Arc;

pub trait Cache: Clone + Send + Sync + 'static {}

impl Cache for Arc<GlobalBlockCache> {}
impl Cache for GlobalBlockCache {}

/// A no-op cache for testing purposes
#[derive(Clone, Debug, Default)]
pub struct NoopCache;

impl Cache for NoopCache {}

impl NoopCache {
    pub fn get(&self, _table_id: u64, _block_idx: usize) -> Option<Vec<u8>> {
        None
    }

    pub fn put(&self, _table_id: u64, _block_idx: usize, _data: Vec<u8>) {
        // No-op
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats { len: 0, cap: 0 }
    }
}

type BlockId = (u64, usize);

const NUM_SHARDS: usize = 16;

type Shard = Mutex<LruCache<BlockId, Vec<u8>>>;

/// A sharded block cache that splits entries across `NUM_SHARDS` independent
/// LRU caches, each protected by its own `parking_lot::Mutex`. This reduces
/// contention under high concurrency compared to a single global lock.
#[derive(Clone, Debug)]
pub struct GlobalBlockCache {
    shards: Arc<[Shard]>,
}

impl GlobalBlockCache {
    pub fn new(size_mb: usize, block_size: usize) -> Arc<Self> {
        let max_blocks = (size_mb * 1024 * 1024) / block_size;
        let total_cap = max_blocks.max(1);
        let per_shard = (total_cap / NUM_SHARDS).max(1);

        let shards: Vec<Shard> = (0..NUM_SHARDS)
            .map(|_| {
                let cap = NonZeroUsize::new(per_shard).expect("per_shard is at least 1");
                Mutex::new(LruCache::new(cap))
            })
            .collect();

        Arc::new(Self {
            shards: shards.into(),
        })
    }

    /// Determines which shard a given (table_id, block_idx) pair maps to.
    fn shard_index(table_id: u64, block_idx: usize) -> usize {
        (table_id as usize ^ block_idx) % NUM_SHARDS
    }

    pub fn get(&self, table_id: u64, block_idx: usize) -> Option<Vec<u8>> {
        let idx = Self::shard_index(table_id, block_idx);
        let mut shard = self.shards[idx].lock();
        shard.get(&(table_id, block_idx)).cloned()
    }

    pub fn put(&self, table_id: u64, block_idx: usize, data: Vec<u8>) {
        let idx = Self::shard_index(table_id, block_idx);
        let mut shard = self.shards[idx].lock();
        shard.put((table_id, block_idx), data);
    }

    pub fn stats(&self) -> CacheStats {
        let mut total_len = 0;
        let mut total_cap = 0;
        for shard in self.shards.iter() {
            let guard = shard.lock();
            total_len += guard.len();
            total_cap += guard.cap().get();
        }
        CacheStats {
            len: total_len,
            cap: total_cap,
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
        // 16 shards, each with capacity 1
        assert_eq!(stats.cap, 16);

        cache.put(1, 0, vec![1]);
        assert_eq!(cache.get(1, 0), Some(vec![1]));

        // Key 16 maps to the same shard as key 0 for table_id=1
        // (1 ^ 0) % 16 == 1 == (1 ^ 16) % 16
        cache.put(1, 16, vec![2]);
        assert_eq!(cache.get(1, 16), Some(vec![2]));
        assert_eq!(cache.get(1, 0), None);
    }

    #[test]
    fn test_cache_large_blocks() {
        let cache = GlobalBlockCache::new(1, 1024 * 1024);
        let stats = cache.stats();
        // 16 shards, each with capacity 1
        assert_eq!(stats.cap, 16);

        let large_block = vec![0u8; 1024 * 1024];
        cache.put(1, 0, large_block.clone());
        assert_eq!(cache.get(1, 0), Some(large_block));
    }

    #[test]
    fn test_cache_concurrent_access() {
        use std::thread;

        let cache = GlobalBlockCache::new(1, 4096);

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

    #[test]
    fn test_cache_shard_distribution() {
        let cache = GlobalBlockCache::new(0, 4096);
        // Each shard has capacity 1, 16 shards total

        // Insert 16 keys, each mapping to a different shard
        // For table_id=0, key i maps to shard (0 ^ i) % 16 = i % 16
        for i in 0..16 {
            cache.put(0, i, vec![i as u8]);
        }

        // All 16 keys should be present (one per shard)
        for i in 0..16 {
            assert_eq!(cache.get(0, i), Some(vec![i as u8]));
        }

        let stats = cache.stats();
        assert_eq!(stats.len, 16);

        // Insert key 16 which maps to shard 0 (same as key 0)
        // This should evict key 0 from shard 0
        cache.put(0, 16, vec![16]);

        // Key 0 should be evicted
        assert_eq!(cache.get(0, 0), None);
        // Key 16 should be present
        assert_eq!(cache.get(0, 16), Some(vec![16]));
        // Keys in other shards should survive
        assert_eq!(cache.get(0, 1), Some(vec![1]));
        assert_eq!(cache.get(0, 15), Some(vec![15]));

        let stats = cache.stats();
        assert_eq!(stats.len, 16); // Still 16 total (one evicted, one added)
    }
}
