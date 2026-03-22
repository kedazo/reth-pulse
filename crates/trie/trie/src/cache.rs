//! Shared trie node cache for reducing MDBX reads during multiproof computation.
//!
//! This module provides a thread-safe, bounded in-memory cache for trie branch nodes.
//! The cache sits between proof workers and MDBX, eliminating redundant disk reads for
//! frequently accessed trie nodes (especially the top levels of the account trie).
//!
//! Cache consistency is maintained via selective invalidation using [`TrieUpdates`]:
//! when a block produces trie updates, only changed/removed nodes are invalidated.

use crate::{updates::TrieUpdates, BranchNodeCompact, Nibbles};
use alloy_primitives::B256;
use reth_primitives_traits::dashmap::DashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// A thread-safe, bounded cache for trie branch nodes shared across proof workers.
///
/// Uses [`DashMap`] for concurrent read/write access. Eviction is a simple capacity cap:
/// new inserts are skipped when the cache is full. This works well because the most
/// frequently accessed nodes (top-level account trie) fill first and are never evicted.
///
/// Selective invalidation via [`apply_updates`](Self::apply_updates) keeps the cache
/// consistent with the trie state between blocks.
#[derive(Debug)]
pub struct SharedTrieNodeCache {
    /// Cached account trie branch nodes, keyed by nibble path.
    account_nodes: DashMap<Nibbles, BranchNodeCompact>,
    /// Cached storage trie branch nodes, keyed by hashed address then nibble path.
    storage_nodes: DashMap<B256, DashMap<Nibbles, BranchNodeCompact>>,
    /// Maximum number of account node entries.
    max_account_entries: usize,
    /// Maximum total number of storage node entries (across all addresses).
    max_storage_entries: usize,
    /// Current count of account node entries.
    account_count: AtomicUsize,
    /// Current count of storage node entries (across all addresses).
    storage_count: AtomicUsize,
}

impl SharedTrieNodeCache {
    /// Creates a new cache with the given capacity limits.
    pub fn new(max_account_entries: usize, max_storage_entries: usize) -> Self {
        Self {
            account_nodes: DashMap::default(),
            storage_nodes: DashMap::default(),
            max_account_entries,
            max_storage_entries,
            account_count: AtomicUsize::new(0),
            storage_count: AtomicUsize::new(0),
        }
    }

    /// Looks up an account trie node by its nibble path. Returns a clone on hit.
    pub fn get_account(&self, key: &Nibbles) -> Option<BranchNodeCompact> {
        self.account_nodes.get(key).map(|entry| entry.value().clone())
    }

    /// Inserts an account trie node. Skips if at capacity.
    pub fn insert_account(&self, key: Nibbles, node: BranchNodeCompact) {
        // Use insert-if-absent to avoid double-counting
        if self.account_nodes.contains_key(&key) {
            // Update existing entry without changing count
            self.account_nodes.insert(key, node);
            return;
        }
        if self.account_count.load(Ordering::Relaxed) >= self.max_account_entries {
            return;
        }
        if self.account_nodes.insert(key, node).is_none() {
            self.account_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Looks up a storage trie node by hashed address and nibble path. Returns a clone on hit.
    pub fn get_storage(&self, address: &B256, key: &Nibbles) -> Option<BranchNodeCompact> {
        self.storage_nodes
            .get(address)
            .and_then(|addr_map| addr_map.get(key).map(|entry| entry.value().clone()))
    }

    /// Inserts a storage trie node. Skips if at capacity.
    pub fn insert_storage(&self, address: B256, key: Nibbles, node: BranchNodeCompact) {
        let addr_map = self.storage_nodes.entry(address).or_default();
        if addr_map.contains_key(&key) {
            addr_map.insert(key, node);
            return;
        }
        if self.storage_count.load(Ordering::Relaxed) >= self.max_storage_entries {
            return;
        }
        if addr_map.insert(key, node).is_none() {
            self.storage_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Applies trie updates to the cache: inserts updated nodes, removes deleted nodes,
    /// and wipes storage tries marked as deleted.
    ///
    /// This is better than pure invalidation — the cache is proactively populated with
    /// the newest values from the latest block's trie computation.
    pub fn apply_updates(&self, updates: &TrieUpdates) {
        // Insert/update account nodes
        for (key, node) in &updates.account_nodes {
            // These are fresh values, so always insert regardless of capacity
            if self.account_nodes.insert(key.clone(), node.clone()).is_none() {
                self.account_count.fetch_add(1, Ordering::Relaxed);
            }
        }

        // Remove deleted account nodes
        for key in &updates.removed_nodes {
            if self.account_nodes.remove(key).is_some() {
                self.account_count.fetch_sub(1, Ordering::Relaxed);
            }
        }

        // Process storage trie updates
        for (address, storage_updates) in &updates.storage_tries {
            if storage_updates.is_deleted {
                // Wipe entire storage trie for this address
                if let Some((_, removed_map)) = self.storage_nodes.remove(address) {
                    self.storage_count.fetch_sub(removed_map.len(), Ordering::Relaxed);
                }
                continue;
            }

            // Insert/update storage nodes
            let addr_map = self.storage_nodes.entry(*address).or_default();
            for (key, node) in &storage_updates.storage_nodes {
                if addr_map.insert(key.clone(), node.clone()).is_none() {
                    self.storage_count.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Remove deleted storage nodes
            for key in &storage_updates.removed_nodes {
                if addr_map.remove(key).is_some() {
                    self.storage_count.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Clears all entries from the cache.
    pub fn clear(&self) {
        self.account_nodes.clear();
        self.storage_nodes.clear();
        self.account_count.store(0, Ordering::Relaxed);
        self.storage_count.store(0, Ordering::Relaxed);
    }

    /// Returns the current (account_count, storage_count).
    pub fn len(&self) -> (usize, usize) {
        (
            self.account_count.load(Ordering::Relaxed),
            self.storage_count.load(Ordering::Relaxed),
        )
    }

    /// Returns true if both account and storage caches are empty.
    pub fn is_empty(&self) -> bool {
        let (a, s) = self.len();
        a == 0 && s == 0
    }
}

/// Creates a new [`SharedTrieNodeCache`] wrapped in an [`Arc`].
pub fn shared_trie_node_cache(
    max_account_entries: usize,
    max_storage_entries: usize,
) -> Arc<SharedTrieNodeCache> {
    Arc::new(SharedTrieNodeCache::new(max_account_entries, max_storage_entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::StorageTrieUpdates;
    use alloy_primitives::map::{HashMap, HashSet};

    fn make_branch_node() -> BranchNodeCompact {
        BranchNodeCompact::new(0b1111, 0, 0, Vec::new(), None)
    }

    fn make_branch_node_2() -> BranchNodeCompact {
        BranchNodeCompact::new(0b1010, 0, 0, Vec::new(), None)
    }

    #[test]
    fn test_account_insert_and_get() {
        let cache = SharedTrieNodeCache::new(100, 100);
        let key = Nibbles::from_nibbles([0x1, 0x2]);
        let node = make_branch_node();

        cache.insert_account(key, node.clone());
        let result = cache.get_account(&key);

        assert_eq!(result, Some(node));
        assert_eq!(cache.len(), (1, 0));
    }

    #[test]
    fn test_account_cache_miss() {
        let cache = SharedTrieNodeCache::new(100, 100);
        let key = Nibbles::from_nibbles([0x1, 0x2]);
        assert_eq!(cache.get_account(&key), None);
    }

    #[test]
    fn test_apply_updates_inserts() {
        let cache = SharedTrieNodeCache::new(100, 100);

        let key1 = Nibbles::from_nibbles([0x1]);
        let key2 = Nibbles::from_nibbles([0x2]);
        let node1 = make_branch_node();
        let node2 = make_branch_node_2();

        let mut account_nodes = HashMap::default();
        account_nodes.insert(key1, node1.clone());
        account_nodes.insert(key2, node2.clone());

        let updates = TrieUpdates {
            account_nodes,
            removed_nodes: HashSet::default(),
            storage_tries: Default::default(),
        };

        cache.apply_updates(&updates);

        assert_eq!(cache.get_account(&key1), Some(node1));
        assert_eq!(cache.get_account(&key2), Some(node2));
        assert_eq!(cache.len(), (2, 0));
    }

    #[test]
    fn test_apply_updates_removes() {
        let cache = SharedTrieNodeCache::new(100, 100);

        let key1 = Nibbles::from_nibbles([0x1]);
        let key2 = Nibbles::from_nibbles([0x2]);
        cache.insert_account(key1, make_branch_node());
        cache.insert_account(key2, make_branch_node_2());
        assert_eq!(cache.len(), (2, 0));

        let mut removed_nodes = HashSet::default();
        removed_nodes.insert(key1);

        let updates = TrieUpdates {
            account_nodes: HashMap::default(),
            removed_nodes,
            storage_tries: Default::default(),
        };

        cache.apply_updates(&updates);

        assert_eq!(cache.get_account(&key1), None);
        assert!(cache.get_account(&key2).is_some());
        assert_eq!(cache.len(), (1, 0));
    }

    #[test]
    fn test_storage_insert_and_get() {
        let cache = SharedTrieNodeCache::new(100, 100);
        let address = B256::with_last_byte(1);
        let key = Nibbles::from_nibbles([0x3, 0x4]);
        let node = make_branch_node();

        cache.insert_storage(address, key, node.clone());
        let result = cache.get_storage(&address, &key);

        assert_eq!(result, Some(node));
        assert_eq!(cache.len(), (0, 1));
    }

    #[test]
    fn test_storage_is_deleted() {
        let cache = SharedTrieNodeCache::new(100, 100);
        let address = B256::with_last_byte(1);

        cache.insert_storage(address, Nibbles::from_nibbles([0x1]), make_branch_node());
        cache.insert_storage(address, Nibbles::from_nibbles([0x2]), make_branch_node_2());
        assert_eq!(cache.len(), (0, 2));

        let mut storage_tries = alloy_primitives::map::B256Map::default();
        storage_tries.insert(
            address,
            StorageTrieUpdates {
                is_deleted: true,
                storage_nodes: HashMap::default(),
                removed_nodes: HashSet::default(),
            },
        );

        let updates = TrieUpdates {
            account_nodes: HashMap::default(),
            removed_nodes: HashSet::default(),
            storage_tries,
        };

        cache.apply_updates(&updates);

        assert_eq!(cache.get_storage(&address, &Nibbles::from_nibbles([0x1])), None);
        assert_eq!(cache.get_storage(&address, &Nibbles::from_nibbles([0x2])), None);
        assert_eq!(cache.len(), (0, 0));
    }

    #[test]
    fn test_capacity_limit() {
        let cache = SharedTrieNodeCache::new(2, 2);

        cache.insert_account(Nibbles::from_nibbles([0x1]), make_branch_node());
        cache.insert_account(Nibbles::from_nibbles([0x2]), make_branch_node());
        cache.insert_account(Nibbles::from_nibbles([0x3]), make_branch_node());

        assert_eq!(cache.len().0, 2);
        // Third insert was skipped
        assert_eq!(cache.get_account(&Nibbles::from_nibbles([0x3])), None);
    }
}
