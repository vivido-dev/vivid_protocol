//! Bounded idempotency-result storage for one logical session (core §5.2).
//!
//! A mutation that carries an envelope idempotency key applies at most once. [`IdempotencyCache`]
//! reserves the key, stores the reply when the mutation completes, and replays that reply for an
//! exact retry; reusing a key for a different request is a conflict.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredResult {
    Pending,
    Complete(Vec<u8>),
    UnknownOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    request_hash: [u8; 32],
    result: StoredResult,
}

/// What [`IdempotencyCache::begin`] found for a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginResult {
    /// The key is new; apply the request, then complete it.
    Fresh,
    /// The request already completed; send this stored reply.
    Replay(Vec<u8>),
    /// The same request is still being applied.
    Pending,
    /// The request may have applied, but no replayable result exists.
    UnknownOutcome,
    /// The key was used for a different request.
    ConflictingReuse,
    /// The cache is full.
    LimitExceeded,
}

/// Why a result could not be stored for a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompletionError {
    /// The key was never reserved.
    NotReserved,
    /// The key already holds a result.
    AlreadyComplete,
}

impl std::fmt::Display for CompletionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotReserved => "idempotency key has no pending reservation",
            Self::AlreadyComplete => "idempotency key already has an outcome",
        })
    }
}

impl std::error::Error for CompletionError {}

/// Bounded results of idempotent mutations, keyed by envelope idempotency key (core §5.2).
#[derive(Debug)]
pub struct IdempotencyCache {
    maximum_entries: usize,
    entries: BTreeMap<[u8; 16], Entry>,
}

impl IdempotencyCache {
    /// Creates an empty cache holding at most `maximum_entries` keys.
    pub fn new(maximum_entries: usize) -> Self {
        Self {
            maximum_entries,
            entries: BTreeMap::new(),
        }
    }

    /// Reserves `key` for `complete_request`, or reports what an earlier use left behind.
    pub fn begin(&mut self, key: [u8; 16], complete_request: &[u8]) -> BeginResult {
        let request_hash: [u8; 32] = Sha256::digest(complete_request).into();
        if let Some(entry) = self.entries.get(&key) {
            if entry.request_hash != request_hash {
                return BeginResult::ConflictingReuse;
            }
            return match &entry.result {
                StoredResult::Pending => BeginResult::Pending,
                StoredResult::Complete(result) => BeginResult::Replay(result.clone()),
                StoredResult::UnknownOutcome => BeginResult::UnknownOutcome,
            };
        }
        if self.entries.len() >= self.maximum_entries {
            return BeginResult::LimitExceeded;
        }
        self.entries.insert(
            key,
            Entry {
                request_hash,
                result: StoredResult::Pending,
            },
        );
        BeginResult::Fresh
    }

    /// Store a non-secret logical result after the mutation outcome is known.
    ///
    /// # Errors
    ///
    /// Returns [`CompletionError::NotReserved`] when `key` has no reservation, and
    /// [`CompletionError::AlreadyComplete`] when it already has an outcome.
    pub fn complete(&mut self, key: [u8; 16], result: Vec<u8>) -> Result<(), CompletionError> {
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or(CompletionError::NotReserved)?;
        if !matches!(entry.result, StoredResult::Pending) {
            return Err(CompletionError::AlreadyComplete);
        }
        entry.result = StoredResult::Complete(result);
        Ok(())
    }

    /// Records that the mutation for `key` ended with an unknown outcome.
    ///
    /// # Errors
    ///
    /// Returns [`CompletionError::NotReserved`] when `key` has no reservation, and
    /// [`CompletionError::AlreadyComplete`] when it already has an outcome.
    pub fn mark_unknown(&mut self, key: [u8; 16]) -> Result<(), CompletionError> {
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or(CompletionError::NotReserved)?;
        if !matches!(entry.result, StoredResult::Pending) {
            return Err(CompletionError::AlreadyComplete);
        }
        entry.result = StoredResult::UnknownOutcome;
        Ok(())
    }

    /// Returns how many keys the cache holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the cache holds no keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Final logical-session cleanup. Suspension intentionally does not call this.
    pub fn clear_on_close(&mut self) {
        self.entries.clear();
    }
}
