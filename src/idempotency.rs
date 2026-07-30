//! Bounded idempotency-result storage scoped to one logical session.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginResult {
    Fresh,
    Replay(Vec<u8>),
    Pending,
    UnknownOutcome,
    ConflictingReuse,
    LimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionError {
    NotReserved,
    AlreadyComplete,
}

#[derive(Debug)]
pub struct IdempotencyCache {
    maximum_entries: usize,
    entries: BTreeMap<[u8; 16], Entry>,
}

impl IdempotencyCache {
    pub fn new(maximum_entries: usize) -> Self {
        Self {
            maximum_entries,
            entries: BTreeMap::new(),
        }
    }

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

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Final logical-session cleanup. Suspension intentionally does not call this.
    pub fn clear_on_close(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_retry_replays_without_double_admission() {
        let mut cache = IdempotencyCache::new(1);
        assert_eq!(cache.begin([1; 16], b"request"), BeginResult::Fresh);
        cache.complete([1; 16], b"result".to_vec()).unwrap();
        assert_eq!(
            cache.begin([1; 16], b"request"),
            BeginResult::Replay(b"result".to_vec())
        );
        assert_eq!(
            cache.begin([1; 16], b"different"),
            BeginResult::ConflictingReuse
        );
        assert_eq!(cache.begin([2; 16], b"request"), BeginResult::LimitExceeded);
    }

    #[test]
    fn uncertain_pending_result_is_explicit_after_resume() {
        let mut cache = IdempotencyCache::new(1);
        assert_eq!(cache.begin([1; 16], b"request"), BeginResult::Fresh);
        cache.mark_unknown([1; 16]).unwrap();
        assert_eq!(
            cache.begin([1; 16], b"request"),
            BeginResult::UnknownOutcome
        );
    }
}
