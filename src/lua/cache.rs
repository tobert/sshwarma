//! Tool result caching for Lua HUD
//!
//! Provides instant access to tool results for HUD rendering.
//! Background tasks update the cache; Lua reads synchronously.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

/// A cached tool result with timestamp
#[derive(Debug, Clone)]
pub struct CachedResult {
    /// The cached data
    pub data: Value,
    /// When this result was fetched
    pub fetched_at: Instant,
}

impl CachedResult {
    /// Create a new cached result with current timestamp
    pub fn new(data: Value) -> Self {
        Self {
            data,
            fetched_at: Instant::now(),
        }
    }

    /// Age of this cached result in seconds
    pub fn age_secs(&self) -> f64 {
        self.fetched_at.elapsed().as_secs_f64()
    }

    /// Check if this result is stale (older than given seconds)
    pub fn is_stale(&self, max_age_secs: f64) -> bool {
        self.age_secs() > max_age_secs
    }
}

/// Thread-safe cache for tool results
///
/// The cache allows background tasks to update results while the Lua
/// runtime reads synchronously during HUD rendering.
#[derive(Debug, Clone)]
pub struct ToolCache {
    cache: Arc<RwLock<HashMap<String, CachedResult>>>,
}

impl ToolCache {
    /// Create a new empty tool cache
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get a cached value by key (instant read)
    ///
    /// Returns None if the key doesn't exist.
    /// Use this for synchronous access from Lua.
    pub async fn get(&self, key: &str) -> Option<CachedResult> {
        self.cache.read().await.get(key).cloned()
    }

    /// Get a cached value synchronously (blocking)
    ///
    /// This is safe to call from non-async contexts.
    /// Returns None if the key doesn't exist.
    pub fn get_blocking(&self, key: &str) -> Option<CachedResult> {
        // Use try_read to avoid blocking if possible
        if let Ok(guard) = self.cache.try_read() {
            guard.get(key).cloned()
        } else {
            // Fallback: If we can't get a read lock, return None
            // This prevents deadlocks in tight loops
            None
        }
    }

    /// Get just the data portion of a cached value
    pub fn get_data_blocking(&self, key: &str) -> Option<Value> {
        self.get_blocking(key).map(|r| r.data)
    }

    /// Set a cached value
    pub async fn set(&self, key: String, value: Value) {
        self.cache
            .write()
            .await
            .insert(key, CachedResult::new(value));
    }

    /// Set a cached value synchronously (blocking)
    pub fn set_blocking(&self, key: String, value: Value) {
        if let Ok(mut guard) = self.cache.try_write() {
            guard.insert(key, CachedResult::new(value));
        }
        // If we can't get the lock, skip the update
        // The next refresh cycle will catch it
    }

    /// Remove a cached value synchronously (blocking)
    pub fn remove_blocking(&self, key: &str) -> Option<Value> {
        if let Ok(mut guard) = self.cache.try_write() {
            guard.remove(key).map(|r| r.data)
        } else {
            None
        }
    }

    /// Remove a cached value
    pub async fn remove(&self, key: &str) -> Option<CachedResult> {
        self.cache.write().await.remove(key)
    }

    /// Clear all cached values
    pub async fn clear(&self) {
        self.cache.write().await.clear();
    }

    /// Get all cached keys
    pub async fn keys(&self) -> Vec<String> {
        self.cache.read().await.keys().cloned().collect()
    }

    /// Get cache size
    pub async fn len(&self) -> usize {
        self.cache.read().await.len()
    }

    /// Check if cache is empty
    pub async fn is_empty(&self) -> bool {
        self.cache.read().await.is_empty()
    }

    /// Remove all stale entries older than given seconds
    pub async fn evict_stale(&self, max_age_secs: f64) {
        self.cache
            .write()
            .await
            .retain(|_, v| !v.is_stale(max_age_secs));
    }

    /// Get a snapshot of all cached data as a HashMap
    ///
    /// Useful for bulk access from Lua
    pub fn snapshot_blocking(&self) -> HashMap<String, Value> {
        if let Ok(guard) = self.cache.try_read() {
            guard
                .iter()
                .map(|(k, v)| (k.clone(), v.data.clone()))
                .collect()
        } else {
            HashMap::new()
        }
    }
}

impl Default for ToolCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_cache_get_set() {
        let cache = ToolCache::new();

        cache
            .set("test_key".to_string(), json!({"foo": "bar"}))
            .await;

        let result = cache.get("test_key").await;
        assert!(result.is_some());

        let cached = result.unwrap();
        assert_eq!(cached.data, json!({"foo": "bar"}));
    }

    #[tokio::test]
    async fn test_cache_blocking() {
        let cache = ToolCache::new();

        cache.set_blocking("sync_key".to_string(), json!(42));

        let result = cache.get_blocking("sync_key");
        assert!(result.is_some());
        assert_eq!(result.unwrap().data, json!(42));
    }

    #[tokio::test]
    async fn test_cache_evict_stale() {
        let cache = ToolCache::new();

        cache.set("key1".to_string(), json!(1)).await;
        cache.set("key2".to_string(), json!(2)).await;

        // Nothing should be stale immediately
        cache.evict_stale(0.001).await;

        // Both keys should still exist (evict_stale uses > not >=)
        assert_eq!(cache.len().await, 2);
    }

    #[test]
    fn test_cached_result_age() {
        let result = CachedResult::new(json!(null));

        // Age should be very small
        assert!(result.age_secs() < 0.1);

        // Should not be stale with large max age
        assert!(!result.is_stale(1000.0));
    }
}
