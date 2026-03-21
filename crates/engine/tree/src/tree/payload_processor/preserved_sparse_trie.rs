//! Preserved sparse trie for reuse across payload validations.

use alloy_primitives::B256;
use parking_lot::Mutex;
use reth_trie_sparse::SparseStateTrie;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;
use tracing::debug;

/// Type alias for the sparse trie type used in preservation.
pub(super) type SparseTrie = SparseStateTrie;

/// Shared handle to a preserved sparse trie that can be reused across payload validations.
///
/// This is stored in [`PayloadProcessor`](super::PayloadProcessor) and cloned to pass to
/// [`SparseTrieCacheTask`](super::sparse_trie::SparseTrieCacheTask) for trie reuse.
#[derive(Debug, Clone)]
pub(super) struct SharedPreservedSparseTrie {
    trie: Arc<Mutex<Option<PreservedSparseTrie>>>,
    /// Set to true while a sparse trie task is running. Used by `take_or_wait()`
    /// to decide whether to wait for the trie to become available.
    task_in_flight: Arc<AtomicBool>,
}

impl Default for SharedPreservedSparseTrie {
    fn default() -> Self {
        Self {
            trie: Arc::new(Mutex::new(None)),
            task_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl SharedPreservedSparseTrie {
    /// Takes the preserved trie if present. If `None` and a task is currently
    /// in flight (computing), waits up to `max_wait` for the trie to become
    /// available before giving up.
    ///
    /// This prevents the new task from starting cold when the previous task
    /// hasn't finished storing the trie yet.
    pub(super) fn take_or_wait(
        &self,
        max_wait: std::time::Duration,
    ) -> Option<PreservedSparseTrie> {
        // Fast path: trie is available
        if let Some(trie) = self.trie.lock().take() {
            return Some(trie);
        }

        // If no task is in flight, the trie is genuinely empty (e.g., first block)
        if !self.task_in_flight.load(Ordering::Acquire) {
            return None;
        }

        // A task is in flight — wait for it to store the trie
        let start = Instant::now();
        while start.elapsed() < max_wait {
            std::thread::sleep(std::time::Duration::from_millis(5));

            if let Some(trie) = self.trie.lock().take() {
                debug!(
                    target: "engine::tree::payload_processor",
                    waited=?start.elapsed(),
                    "Retrieved trie after waiting for in-flight task"
                );
                return Some(trie);
            }

            // Task finished without storing (error case)
            if !self.task_in_flight.load(Ordering::Acquire) {
                return None;
            }
        }

        debug!(
            target: "engine::tree::payload_processor",
            waited=?max_wait,
            "Timed out waiting for in-flight task to store trie"
        );
        None
    }

    /// Marks that a task has started computing.
    pub(super) fn set_task_in_flight(&self) {
        self.task_in_flight.store(true, Ordering::Release);
    }

    /// Marks that a task has finished computing.
    pub(super) fn clear_task_in_flight(&self) {
        self.task_in_flight.store(false, Ordering::Release);
    }

    /// Acquires a guard that blocks `take()` until dropped.
    /// Use this before sending the state root result to ensure the next block
    /// waits for the trie to be stored.
    pub(super) fn lock(&self) -> PreservedTrieGuard<'_> {
        PreservedTrieGuard(self.trie.lock())
    }

    /// Waits until the sparse trie lock becomes available.
    ///
    /// This acquires and immediately releases the lock, ensuring that any
    /// ongoing operations complete before returning. Useful for synchronization
    /// before starting payload processing.
    ///
    /// Returns the time spent waiting for the lock.
    pub(super) fn wait_for_availability(&self) -> std::time::Duration {
        let start = Instant::now();
        let _guard = self.trie.lock();
        let elapsed = start.elapsed();
        if elapsed.as_millis() > 5 {
            debug!(
                target: "engine::tree::payload_processor",
                blocked_for=?elapsed,
                "Waited for preserved sparse trie to become available"
            );
        }
        elapsed
    }
}

/// Guard that holds the lock on the preserved trie.
/// While held, `take()` will block. Call `store()` to save the trie before dropping.
pub(super) struct PreservedTrieGuard<'a>(parking_lot::MutexGuard<'a, Option<PreservedSparseTrie>>);

impl PreservedTrieGuard<'_> {
    /// Stores a preserved trie for later reuse.
    pub(super) fn store(&mut self, trie: PreservedSparseTrie) {
        self.0.replace(trie);
    }
}

/// A preserved sparse trie that can be reused across payload validations.
///
/// The trie exists in one of two states:
/// - **Anchored**: Has a computed state root and can be reused for payloads whose parent state root
///   matches the anchor.
/// - **Cleared**: Trie data has been cleared but allocations are preserved for reuse.
#[derive(Debug)]
pub(super) enum PreservedSparseTrie {
    /// Trie with a computed state root that can be reused for continuation payloads.
    Anchored {
        /// The sparse state trie (pruned after root computation).
        trie: SparseTrie,
        /// The state root this trie represents (computed from the previous block).
        /// Used to verify continuity: new payload's `parent_state_root` must match this.
        state_root: B256,
    },
    /// Cleared trie with preserved allocations, ready for fresh use.
    Cleared {
        /// The sparse state trie with cleared data but preserved allocations.
        trie: SparseTrie,
    },
}

impl PreservedSparseTrie {
    /// Creates a new anchored preserved trie.
    ///
    /// The `state_root` is the computed state root from the trie, which becomes the
    /// anchor for determining if subsequent payloads can reuse this trie.
    pub(super) const fn anchored(trie: SparseTrie, state_root: B256) -> Self {
        Self::Anchored { trie, state_root }
    }

    /// Creates a cleared preserved trie (allocations preserved, data cleared).
    pub(super) const fn cleared(trie: SparseTrie) -> Self {
        Self::Cleared { trie }
    }

    /// Consumes self and returns the trie for reuse.
    ///
    /// If the preserved trie is anchored and the parent state root matches, the pruned
    /// trie structure is reused directly. Otherwise, the trie is cleared but allocations
    /// are preserved to reduce memory overhead.
    pub(super) fn into_trie_for(self, parent_state_root: B256) -> SparseTrie {
        match self {
            Self::Anchored { trie, state_root } if state_root == parent_state_root => {
                debug!(
                    target: "engine::tree::payload_processor",
                    %state_root,
                    "Reusing anchored sparse trie for continuation payload"
                );
                trie
            }
            Self::Anchored { mut trie, state_root } => {
                debug!(
                    target: "engine::tree::payload_processor",
                    anchor_root = %state_root,
                    %parent_state_root,
                    "Clearing anchored sparse trie - parent state root mismatch"
                );
                trie.clear();
                trie
            }
            Self::Cleared { trie } => {
                debug!(
                    target: "engine::tree::payload_processor",
                    %parent_state_root,
                    "Using cleared sparse trie with preserved allocations"
                );
                trie
            }
        }
    }
}
