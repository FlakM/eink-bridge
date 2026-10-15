use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;

use super::race::Recognition;

pub(super) type SharedRecognition = Arc<OnceCell<Result<Recognition, String>>>;

/// Exact PNG keys avoid hash-collision false hits. Bounds apply to keys and
/// entry count; successful text is separately limited to 8 KiB per entry.
pub(super) struct RecognitionCache {
    entries: HashMap<Vec<u8>, Entry>,
    capacity: usize,
    max_bytes: usize,
    ttl: Duration,
}

struct Entry {
    used: Instant,
    value: SharedRecognition,
}

impl Default for RecognitionCache {
    fn default() -> Self {
        Self::new(128, 16 * 1024 * 1024, Duration::from_secs(600))
    }
}

impl RecognitionCache {
    pub(super) fn new(capacity: usize, max_bytes: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            max_bytes,
            ttl,
        }
    }

    pub(super) fn entry(&mut self, png: &[u8], now: Instant) -> SharedRecognition {
        self.entries
            .retain(|_, entry| now.duration_since(entry.used) < self.ttl);
        if let Some(entry) = self.entries.get_mut(png) {
            entry.used = now;
            return entry.value.clone();
        }
        let value = Arc::new(OnceCell::new());
        if self.capacity == 0 || self.ttl.is_zero() || png.len() > self.max_bytes {
            return value;
        }
        let mut bytes: usize = self.entries.keys().map(Vec::len).sum();
        while self.entries.len() >= self.capacity || bytes + png.len() > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else { break };
            bytes -= oldest.len();
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            png.to_vec(),
            Entry {
                used: now,
                value: value.clone(),
            },
        );
        value
    }

    pub(super) fn discard(&mut self, png: &[u8], value: &SharedRecognition) {
        if self
            .entries
            .get(png)
            .is_some_and(|entry| Arc::ptr_eq(&entry.value, value))
        {
            self.entries.remove(png);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_images_share_entries_but_edits_do_not() {
        let mut cache = RecognitionCache::default();
        let now = Instant::now();
        let first = cache.entry(b"ink", now);
        assert!(Arc::ptr_eq(&first, &cache.entry(b"ink", now)));
        assert!(!Arc::ptr_eq(&first, &cache.entry(b"ink edited", now)));
    }

    #[test]
    fn entries_expire_and_least_recently_used_is_evicted() {
        let mut cache = RecognitionCache::new(2, 1024, Duration::from_secs(10));
        let now = Instant::now();
        let first = cache.entry(b"one", now);
        let second = cache.entry(b"two", now);
        cache.entry(b"one", now + Duration::from_secs(1));
        cache.entry(b"three", now + Duration::from_secs(2));
        assert!(Arc::ptr_eq(
            &first,
            &cache.entry(b"one", now + Duration::from_secs(3))
        ));
        assert!(!Arc::ptr_eq(
            &second,
            &cache.entry(b"two", now + Duration::from_secs(3))
        ));
        assert!(!Arc::ptr_eq(
            &first,
            &cache.entry(b"one", now + Duration::from_secs(20))
        ));
    }

    #[test]
    fn byte_budget_and_disabled_cache_are_respected() {
        let mut cache = RecognitionCache::new(128, 4, Duration::from_secs(10));
        let now = Instant::now();
        let first = cache.entry(b"123", now);
        cache.entry(b"456", now);
        assert!(!Arc::ptr_eq(&first, &cache.entry(b"123", now)));
        let large = cache.entry(b"12345", now);
        assert!(!Arc::ptr_eq(&large, &cache.entry(b"12345", now)));
        let mut disabled = RecognitionCache::new(0, 4, Duration::from_secs(10));
        assert!(!Arc::ptr_eq(
            &disabled.entry(b"x", now),
            &disabled.entry(b"x", now)
        ));
    }
}
