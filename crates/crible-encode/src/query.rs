// SPDX-License-Identifier: Apache-2.0

//! Encoding of search queries, with an in-memory LRU cache.
//!
//! Queries are encoded in INT8 by default (`[encoder].query_precision`), or in
//! fp32 on request, and fall back to fp32 when the model directory has no INT8
//! graph (`docs/04-donnees-et-ingestion.md`, ADR-0051).
//!
//! The cache is per instance, bounded by `[limits].query_cache_entries`
//! (1 024 entries by default), and **never persisted**: it disappears at
//! shutdown and nothing is written to disk. Its key is
//! `(fingerprint, precision, exact query text)`; the text stored is the one
//! before the prompt (`docs/03-recherche.md`, step 2, ADR-0051). No query text
//! reaches a log or a metric.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crible_core::error::{CribleError, ErrorCode};
use crible_core::model::ModelConfig;

use crate::model::{Precision, Side, read_config};
use crate::registry::ModelRegistry;

/// Default number of cached queries, `[limits].query_cache_entries` of
/// `crible.toml` (ADR-0051). At 768 dimensions, 1 024 fp32 vectors weigh 3,0
/// Mio.
pub const DEFAULT_QUERY_CACHE_ENTRIES: usize = 1_024;

/// A query encoded by [`QueryEncoder`].
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedQuery {
    /// Pooled and normalized vector.
    pub vector: Vec<f32>,
    /// Whether the query exceeded `max_tokens` and lost its tail.
    pub truncated: bool,
    /// Precision the vector was actually produced with: the requested one, or
    /// fp32 after the fallback.
    pub precision: Precision,
}

/// Counters of the query cache, for diagnostics and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that had to encode the query.
    pub misses: u64,
    /// Entries currently held.
    pub entries: usize,
    /// Maximum number of entries.
    pub capacity: usize,
}

/// Encodes queries against one model directory and caches their vectors.
///
/// The registry is shared so that several encoders of one process reuse the
/// loaded sessions. The cache is per encoder; the server creates one per
/// instance and shares it across indexes (ADR-0051).
pub struct QueryEncoder {
    directory: PathBuf,
    registry: Arc<ModelRegistry>,
    cache: Mutex<QueryCache>,
}

impl std::fmt::Debug for QueryEncoder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QueryEncoder")
            .field("directory", &self.directory)
            .field("registry", &self.registry)
            .field("cache", &self.cache)
            .finish()
    }
}

impl QueryEncoder {
    /// Creates an encoder for the model of `directory`.
    ///
    /// `cache_entries` is `[limits].query_cache_entries`; zero disables the
    /// cache. No configuration file is read here: the caller that owns the
    /// instance configuration passes the value, as the CLI does with
    /// [`DEFAULT_QUERY_CACHE_ENTRIES`].
    #[must_use]
    pub fn new(
        directory: impl Into<PathBuf>,
        registry: Arc<ModelRegistry>,
        cache_entries: usize,
    ) -> Self {
        Self {
            directory: directory.into(),
            registry,
            cache: Mutex::new(QueryCache::new(cache_entries)),
        }
    }

    /// Encodes `text` with `precision`, or with fp32 when the model declares no
    /// INT8 graph.
    ///
    /// The fallback covers a model whose `model.json` has no `files.int8`
    /// entry, which is how an index can be created before its quantized graph
    /// exists. A model that **declares** an INT8 file which is missing fails
    /// with [`ErrorCode::InvalidRequest`] instead: the directory is then
    /// damaged, and silently serving another graph would hide the damage. The
    /// hash check of the INT8 file follows the same rule, so a modified or
    /// deleted declared graph is never replaced by fp32.
    ///
    /// The `query` prompt is added by the model, and the token count includes
    /// it. A second call with the same text, model and effective precision is
    /// served from the cache, without a second ONNX run.
    ///
    /// # Errors
    ///
    /// - [`ErrorCode::InvalidRequest`] when `model.json` is malformed, a
    ///   declared graph or the tokenizer is missing or does not match its
    ///   declared hash, the tokenizer fails, or `max_tokens` is zero.
    /// - [`ErrorCode::Internal`] for an I/O failure, ONNX Runtime failure, or a
    ///   poisoned cache.
    pub fn encode_query(
        &self,
        text: &str,
        precision: Precision,
    ) -> Result<EncodedQuery, CribleError> {
        let config = read_config(&self.directory)?;
        let precision = resolve_precision(&config, precision);
        let key = QueryKey {
            fingerprint: config.fingerprint(),
            precision,
            text: text.to_owned(),
        };
        {
            let mut cache = self.cache.lock().map_err(|_| poisoned_cache())?;
            if let Some(entry) = cache.get(&key) {
                return Ok(EncodedQuery {
                    vector: entry.vector.clone(),
                    truncated: entry.truncated,
                    precision,
                });
            }
        }

        // The cache lock is released before the ONNX run: two threads that miss
        // the same query both encode it, which costs one extra run instead of
        // serializing every query of the instance behind one lock.
        let model = self
            .registry
            .get_or_load_precision(&self.directory, precision)?;
        let encoded = model.encode_with_limit(Side::Query, text, config.max_tokens)?;
        let entry = CacheEntry {
            vector: encoded.vector.clone(),
            truncated: encoded.truncated,
            tick: 0,
        };
        let mut cache = self.cache.lock().map_err(|_| poisoned_cache())?;
        cache.insert(key, entry);
        Ok(EncodedQuery {
            vector: encoded.vector,
            truncated: encoded.truncated,
            precision,
        })
    }

    /// Current counters of the cache.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the cache lock is poisoned.
    pub fn cache_stats(&self) -> Result<CacheStats, CribleError> {
        let cache = self.cache.lock().map_err(|_| poisoned_cache())?;
        Ok(cache.stats())
    }
}

/// Applies the INT8 fallback: the requested precision stands unless it is INT8
/// and the model declares no INT8 file.
///
/// Only the **declaration** is looked at here. A declared file that is absent
/// or corrupt is an integrity error raised by [`Model::load_precision`], not a
/// fallback: the directory says it has an INT8 graph, so serving fp32 would hide
/// a broken model.
fn resolve_precision(config: &ModelConfig, requested: Precision) -> Precision {
    let has_int8 = config
        .files
        .as_ref()
        .is_some_and(|files| files.int8.is_some());
    if requested == Precision::Int8 && !has_int8 {
        Precision::Fp32
    } else {
        requested
    }
}

/// Identity of one cached query.
#[derive(Clone, PartialEq, Eq, Hash)]
struct QueryKey {
    fingerprint: String,
    precision: Precision,
    /// Query text without the prompt, exactly as the caller gave it.
    ///
    /// Never printed: [`std::fmt::Debug`] shows `<redacted>`, because a log line
    /// must not carry a query (AGENTS.md, "Confidentialité").
    text: String,
}

impl std::fmt::Debug for QueryKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QueryKey")
            .field("fingerprint", &self.fingerprint)
            .field("precision", &self.precision)
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    vector: Vec<f32>,
    truncated: bool,
    tick: u64,
}

/// Bounded LRU over encoded queries.
///
/// Eviction is a scan of the entries for the smallest tick. A cache holds
/// `query_cache_entries` entries (1 024 by default), and an insertion happens
/// only after a miss that took an ONNX run, so the scan is never on a hot path.
struct QueryCache {
    capacity: usize,
    entries: HashMap<QueryKey, CacheEntry>,
    tick: u64,
    hits: u64,
    misses: u64,
}

impl std::fmt::Debug for QueryCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The entries hold query texts and vectors: only their count is shown.
        formatter
            .debug_struct("QueryCache")
            .field("capacity", &self.capacity)
            .field("entries", &self.entries.len())
            .field("tick", &self.tick)
            .field("hits", &self.hits)
            .field("misses", &self.misses)
            .finish()
    }
}

impl QueryCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: HashMap::new(),
            tick: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Looks a key up and marks it as the most recently used.
    fn get(&mut self, key: &QueryKey) -> Option<&CacheEntry> {
        self.tick += 1;
        let Some(entry) = self.entries.get_mut(key) else {
            self.misses += 1;
            return None;
        };
        entry.tick = self.tick;
        self.hits += 1;
        Some(entry)
    }

    /// Inserts or refreshes an entry, evicting the least recently used one when
    /// the cache is full.
    fn insert(&mut self, key: QueryKey, mut entry: CacheEntry) {
        if self.capacity == 0 {
            return;
        }
        self.tick += 1;
        entry.tick = self.tick;
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.tick)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, entry);
    }

    fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
            capacity: self.capacity,
        }
    }
}

fn poisoned_cache() -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        "the query cache is poisoned by a previous panic",
    )
}

#[cfg(test)]
mod tests {
    use super::{CacheEntry, Precision, QueryCache, QueryKey, resolve_precision};
    use crible_core::model::ModelConfig;

    fn key(text: &str, precision: Precision) -> QueryKey {
        QueryKey {
            fingerprint: "fingerprint".to_owned(),
            precision,
            text: text.to_owned(),
        }
    }

    fn entry(value: f32) -> CacheEntry {
        CacheEntry {
            vector: vec![value],
            truncated: false,
            tick: 0,
        }
    }

    #[test]
    fn serves_the_second_identical_query_from_the_cache() {
        // Acceptance criterion: the cache serves the second identical query.
        let mut cache = QueryCache::new(8);
        let key = key("responsabilité du fait des choses", Precision::Int8);
        assert!(cache.get(&key).is_none(), "the first lookup misses");
        cache.insert(key.clone(), entry(0.5));
        let served = cache.get(&key).expect("the second lookup hits");
        assert_eq!(served.vector, vec![0.5], "the cached vector is returned");
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn keeps_the_two_precisions_of_one_query_apart() {
        let mut cache = QueryCache::new(8);
        let int8 = key("droit", Precision::Int8);
        let fp32 = key("droit", Precision::Fp32);
        cache.insert(int8.clone(), entry(1.0));
        cache.insert(fp32.clone(), entry(2.0));
        assert_eq!(cache.get(&int8).expect("int8 entry").vector, vec![1.0]);
        assert_eq!(cache.get(&fp32).expect("fp32 entry").vector, vec![2.0]);
        assert_eq!(cache.stats().entries, 2, "one entry per precision");
    }

    #[test]
    fn evicts_the_least_recently_used_entry() {
        let mut cache = QueryCache::new(2);
        let first = key("first", Precision::Int8);
        let second = key("second", Precision::Int8);
        let third = key("third", Precision::Int8);
        cache.insert(first.clone(), entry(1.0));
        cache.insert(second.clone(), entry(2.0));
        // Touch the first entry, so the second becomes the least recently used.
        assert!(cache.get(&first).is_some());
        cache.insert(third.clone(), entry(3.0));
        assert!(cache.get(&first).is_some(), "the touched entry survives");
        assert!(cache.get(&second).is_none(), "the oldest entry was evicted");
        assert!(cache.get(&third).is_some(), "the new entry is cached");
        assert_eq!(cache.stats().entries, 2, "the cache stays bounded");
    }

    #[test]
    fn debug_never_prints_a_query_text() {
        // A log line must not carry a query (AGENTS.md, "Confidentialité"), so
        // the derived `Debug` of the key and the cache is replaced.
        let key = key("secret query", Precision::Int8);
        let rendered_key = format!("{key:?}");
        assert!(!rendered_key.contains("secret"), "{rendered_key}");
        assert!(rendered_key.contains("<redacted>"), "{rendered_key}");

        let mut cache = QueryCache::new(8);
        cache.insert(key, entry(1.0));
        let rendered_cache = format!("{cache:?}");
        assert!(!rendered_cache.contains("secret"), "{rendered_cache}");
        assert!(!rendered_cache.contains("QueryKey"), "{rendered_cache}");
    }

    #[test]
    fn zero_capacity_disables_the_cache() {
        let mut cache = QueryCache::new(0);
        let key = key("droit", Precision::Int8);
        cache.insert(key.clone(), entry(1.0));
        assert!(cache.get(&key).is_none(), "nothing is stored");
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.stats().capacity, 0);
    }

    fn config(files: &str) -> ModelConfig {
        let raw = format!(
            r#"{{
  "name": "synthetic",
  "kind": "onnx",
  "hf_id": "lightonai/mDenseOn",
  "revision": "a5fdb000f7a21da96c3bddde3a782ef777316df3",
  "dims": 4,
  "pooling": "cls",
  "normalize": true,
  "max_tokens": 8192,
  "prompts": {{ "query": "query: ", "document": "document: " }},
  "files": {{
    "fp32": {{ "path": "model.onnx", "blake3": "{}" }},
    {files}
    "tokenizer": {{ "path": "tokenizer.json", "blake3": "{}" }}
  }}
}}"#,
            "a".repeat(64),
            "b".repeat(64),
        );
        ModelConfig::from_json_str(&raw).expect("the synthetic configuration is valid")
    }

    #[test]
    fn falls_back_to_fp32_when_the_model_declares_no_int8_file() {
        let without_int8 = config("");
        assert_eq!(
            resolve_precision(&without_int8, Precision::Int8),
            Precision::Fp32,
            "an absent INT8 graph falls back to fp32"
        );
        assert_eq!(
            resolve_precision(&without_int8, Precision::Fp32),
            Precision::Fp32,
            "the fp32 request is unchanged"
        );

        let int8 = format!(
            r#""int8": {{ "path": "model.int8.onnx", "blake3": "{}" }},"#,
            "c".repeat(64)
        );
        let with_int8 = config(&int8);
        assert_eq!(
            resolve_precision(&with_int8, Precision::Int8),
            Precision::Int8,
            "a declared INT8 graph is used"
        );
    }
}
