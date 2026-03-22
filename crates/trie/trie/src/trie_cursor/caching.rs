//! Caching wrappers for trie cursors that use a [`SharedTrieNodeCache`] to reduce MDBX reads.
//!
//! Follows the same wrapper pattern as [`InstrumentedTrieCursor`](super::metrics::InstrumentedTrieCursor).

use super::{TrieCursor, TrieCursorFactory, TrieStorageCursor};
use crate::{
    cache::SharedTrieNodeCache,
    hashed_cursor::HashedCursorFactory,
    BranchNodeCompact, Nibbles,
};
use alloy_primitives::B256;
use reth_storage_errors::db::DatabaseError;
use std::sync::Arc;

/// A wrapper around a [`TrieCursor`] that caches account trie node lookups.
///
/// On `seek_exact`, checks the shared cache first. On miss, delegates to the inner cursor
/// and populates the cache with the result. `seek` and `next` always delegate to the inner
/// cursor (since the returned key differs from the input) but cache results by returned key.
#[derive(Debug)]
pub struct CachingAccountTrieCursor<C> {
    cursor: C,
    cache: Arc<SharedTrieNodeCache>,
}

impl<C> CachingAccountTrieCursor<C> {
    /// Creates a new caching account trie cursor.
    pub const fn new(cursor: C, cache: Arc<SharedTrieNodeCache>) -> Self {
        Self { cursor, cache }
    }
}

impl<C: TrieCursor> TrieCursor for CachingAccountTrieCursor<C> {
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        // Check cache first
        if let Some(node) = self.cache.get_account(&key) {
            return Ok(Some((key, node)));
        }
        // Miss: delegate to inner, cache result
        let result = self.cursor.seek_exact(key)?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_account(k.clone(), v.clone());
        }
        Ok(result)
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        // Can't check cache by input key (seek returns >= key)
        // Delegate to inner, cache result by RETURNED key
        let result = self.cursor.seek(key)?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_account(k.clone(), v.clone());
        }
        Ok(result)
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        let result = self.cursor.next()?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_account(k.clone(), v.clone());
        }
        Ok(result)
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        self.cursor.current()
    }

    fn reset(&mut self) {
        self.cursor.reset()
    }
}

/// A wrapper around a [`TrieStorageCursor`] that caches storage trie node lookups.
///
/// Same caching strategy as [`CachingAccountTrieCursor`] but keyed by `(hashed_address, nibbles)`.
#[derive(Debug)]
pub struct CachingStorageTrieCursor<C> {
    cursor: C,
    cache: Arc<SharedTrieNodeCache>,
    hashed_address: B256,
}

impl<C> CachingStorageTrieCursor<C> {
    /// Creates a new caching storage trie cursor.
    pub const fn new(cursor: C, cache: Arc<SharedTrieNodeCache>, hashed_address: B256) -> Self {
        Self { cursor, cache, hashed_address }
    }
}

impl<C: TrieCursor> TrieCursor for CachingStorageTrieCursor<C> {
    fn seek_exact(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        if let Some(node) = self.cache.get_storage(&self.hashed_address, &key) {
            return Ok(Some((key, node)));
        }
        let result = self.cursor.seek_exact(key)?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_storage(self.hashed_address, k.clone(), v.clone());
        }
        Ok(result)
    }

    fn seek(
        &mut self,
        key: Nibbles,
    ) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        let result = self.cursor.seek(key)?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_storage(self.hashed_address, k.clone(), v.clone());
        }
        Ok(result)
    }

    fn next(&mut self) -> Result<Option<(Nibbles, BranchNodeCompact)>, DatabaseError> {
        let result = self.cursor.next()?;
        if let Some((ref k, ref v)) = result {
            self.cache.insert_storage(self.hashed_address, k.clone(), v.clone());
        }
        Ok(result)
    }

    fn current(&mut self) -> Result<Option<Nibbles>, DatabaseError> {
        self.cursor.current()
    }

    fn reset(&mut self) {
        self.cursor.reset()
    }
}

impl<C: TrieStorageCursor> TrieStorageCursor for CachingStorageTrieCursor<C> {
    fn set_hashed_address(&mut self, hashed_address: B256) {
        self.hashed_address = hashed_address;
        self.cursor.set_hashed_address(hashed_address);
    }
}

/// A factory wrapper that creates caching trie cursors.
///
/// Wraps trie cursors with [`CachingAccountTrieCursor`] / [`CachingStorageTrieCursor`].
/// Delegates [`HashedCursorFactory`] directly (hashed cursors read leaf data, not trie
/// branch nodes, so no caching is needed).
#[derive(Debug, Clone)]
pub struct CachingTrieCursorFactory<F> {
    inner: F,
    cache: Arc<SharedTrieNodeCache>,
}

impl<F> CachingTrieCursorFactory<F> {
    /// Creates a new caching trie cursor factory.
    pub const fn new(inner: F, cache: Arc<SharedTrieNodeCache>) -> Self {
        Self { inner, cache }
    }
}

impl<F: TrieCursorFactory> TrieCursorFactory for CachingTrieCursorFactory<F> {
    type AccountTrieCursor<'a>
        = CachingAccountTrieCursor<F::AccountTrieCursor<'a>>
    where
        Self: 'a;

    type StorageTrieCursor<'a>
        = CachingStorageTrieCursor<F::StorageTrieCursor<'a>>
    where
        Self: 'a;

    fn account_trie_cursor(&self) -> Result<Self::AccountTrieCursor<'_>, DatabaseError> {
        let inner = self.inner.account_trie_cursor()?;
        Ok(CachingAccountTrieCursor::new(inner, self.cache.clone()))
    }

    fn storage_trie_cursor(
        &self,
        hashed_address: B256,
    ) -> Result<Self::StorageTrieCursor<'_>, DatabaseError> {
        let inner = self.inner.storage_trie_cursor(hashed_address)?;
        Ok(CachingStorageTrieCursor::new(inner, self.cache.clone(), hashed_address))
    }
}

impl<F: HashedCursorFactory> HashedCursorFactory for CachingTrieCursorFactory<F> {
    type AccountCursor<'a>
        = F::AccountCursor<'a>
    where
        Self: 'a;

    type StorageCursor<'a>
        = F::StorageCursor<'a>
    where
        Self: 'a;

    fn hashed_account_cursor(&self) -> Result<Self::AccountCursor<'_>, DatabaseError> {
        self.inner.hashed_account_cursor()
    }

    fn hashed_storage_cursor(
        &self,
        hashed_address: B256,
    ) -> Result<Self::StorageCursor<'_>, DatabaseError> {
        self.inner.hashed_storage_cursor(hashed_address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trie_cursor::mock::MockTrieCursorFactory;
    use std::collections::BTreeMap;

    fn make_branch_node() -> BranchNodeCompact {
        BranchNodeCompact::new(0b1111, 0, 0, Vec::new(), None)
    }

    fn make_branch_node_2() -> BranchNodeCompact {
        BranchNodeCompact::new(0b1010, 0, 0, Vec::new(), None)
    }

    #[test]
    fn test_seek_exact_caches_result() {
        let key = Nibbles::from_nibbles([0x1, 0x2]);
        let node = make_branch_node();

        let mut nodes = BTreeMap::new();
        nodes.insert(key, node.clone());
        let factory = MockTrieCursorFactory::new(nodes, Default::default());
        let cache = Arc::new(SharedTrieNodeCache::new(100, 100));

        // First call: cache miss, delegates to mock
        let mut cursor =
            CachingAccountTrieCursor::new(factory.account_trie_cursor().unwrap(), cache.clone());
        let result = cursor.seek_exact(key).unwrap();
        assert_eq!(result, Some((key, node.clone())));

        // Verify it's now in the cache
        assert_eq!(cache.get_account(&key), Some(node.clone()));

        // Second call with a fresh cursor: cache hit, mock not called
        let factory2 = MockTrieCursorFactory::new(BTreeMap::new(), Default::default());
        let mut cursor2 =
            CachingAccountTrieCursor::new(factory2.account_trie_cursor().unwrap(), cache.clone());
        let result2 = cursor2.seek_exact(key).unwrap();
        assert_eq!(result2, Some((key, node)));
    }

    #[test]
    fn test_seek_caches_by_returned_key() {
        let key1 = Nibbles::from_nibbles([0x1]);
        let key2 = Nibbles::from_nibbles([0x2]);
        let node2 = make_branch_node();

        let mut nodes = BTreeMap::new();
        nodes.insert(key2, node2.clone());
        let factory = MockTrieCursorFactory::new(nodes, Default::default());
        let cache = Arc::new(SharedTrieNodeCache::new(100, 100));

        let mut cursor =
            CachingAccountTrieCursor::new(factory.account_trie_cursor().unwrap(), cache.clone());

        // Seek for key1, but key2 is returned (first >= key1)
        let result = cursor.seek(key1).unwrap();
        assert_eq!(result, Some((key2, node2.clone())));

        // Cached under key2, not key1
        assert_eq!(cache.get_account(&key1), None);
        assert_eq!(cache.get_account(&key2), Some(node2));
    }

    #[test]
    fn test_storage_cursor_address_tracking() {
        let addr1 = B256::with_last_byte(1);
        let addr2 = B256::with_last_byte(2);
        let key = Nibbles::from_nibbles([0x5]);
        let node1 = make_branch_node();
        let node2 = make_branch_node_2();

        let mut storage = alloy_primitives::map::B256Map::default();
        let mut s1 = BTreeMap::new();
        s1.insert(key, node1.clone());
        let mut s2 = BTreeMap::new();
        s2.insert(key, node2.clone());
        storage.insert(addr1, s1);
        storage.insert(addr2, s2);

        let factory = MockTrieCursorFactory::new(BTreeMap::new(), storage);
        let cache = Arc::new(SharedTrieNodeCache::new(100, 100));

        let mut cursor = CachingStorageTrieCursor::new(
            factory.storage_trie_cursor(addr1).unwrap(),
            cache.clone(),
            addr1,
        );

        let result = cursor.seek_exact(key).unwrap();
        assert_eq!(result, Some((key, node1.clone())));
        assert_eq!(cache.get_storage(&addr1, &key), Some(node1));

        // Switch to addr2
        cursor.set_hashed_address(addr2);
        let result = cursor.seek_exact(key).unwrap();
        assert_eq!(result, Some((key, node2.clone())));
        assert_eq!(cache.get_storage(&addr2, &key), Some(node2));
    }

    #[test]
    fn test_factory_wraps_cursors() {
        let key = Nibbles::from_nibbles([0x1]);
        let node = make_branch_node();

        let mut nodes = BTreeMap::new();
        nodes.insert(key, node.clone());
        let inner_factory = MockTrieCursorFactory::new(nodes, Default::default());
        let cache = Arc::new(SharedTrieNodeCache::new(100, 100));
        let factory = CachingTrieCursorFactory::new(inner_factory, cache.clone());

        let mut cursor = factory.account_trie_cursor().unwrap();
        let result = cursor.seek_exact(key).unwrap();
        assert_eq!(result, Some((key, node.clone())));
        assert_eq!(cache.get_account(&key), Some(node));
    }
}
