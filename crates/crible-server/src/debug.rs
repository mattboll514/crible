// SPDX-License-Identifier: Apache-2.0

//! The `debug` member of a search answer (ADR-0119): plan, timings and ranks.
//!
//! References: `docs/05-api.md` ("Recherche : réponse", "Recherche
//! multi-index"), `docs/03-recherche.md` (the nine timing keys, the rank of a
//! branch) and ADR-0119 (the shape, the ranks, the timings, the capture and the
//! multi-index members).
//!
//! # One measurement for the response and the analytics
//!
//! The timings rendered here read the [`StageDurations`] the route already
//! records for `AGREGER-ANALYTICS` (ADR-0117, decision 5; ADR-0119,
//! decision 7). No second stopwatch exists: `validate`, `encode` and the steps
//! of the pipeline are measured once, and `total` is the duration `took_ms`
//! reports. A step the mode did not run is `None` in [`StageDurations`] and
//! renders `0.0` here, never a JSON `null` (ADR-0119, decision 5).
//!
//! # Absent members of `per_result`
//!
//! A branch member that has no value is **omitted**, never `null` and never
//! `0` (ADR-0119, decision 3): `dense_rank` and `dense_score` in mode `lexical`,
//! `lexical_rank` and `lexical_score` in mode `dense`, and the members of a
//! branch that did not find the result. `key` and `fused_rank` are always
//! present; `key` is the external `doc_id#ord`, never an `iid` (INV-01).
//!
//! ```text
//! cargo test -p crible-server debug::tests
//! ```

use std::time::Duration;

use crible_core::config::{FusionConfig, SearchConfig};
use crible_search::dense::Strategy;
use crible_search::group::{Grouping, candidates_per_branch};
use crible_search::pipeline::{Mode, RankedHit};
use serde_json::{Map, Value, json};

use crate::analytics::StageDurations;

/// The nine timing keys of `docs/03-recherche.md`, in their order.
pub(crate) const TIMING_KEYS: [&str; 9] = [
    "validate", "encode", "filter", "dense", "lexical", "fusion", "group", "fetch", "total",
];

/// Rounds a duration in milliseconds to three decimals (ADR-0119, decision 5).
#[must_use]
pub(crate) fn round_ms(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

/// Milliseconds of an optional duration; `None` renders `0.0`, not `null`.
fn millis(duration: Option<Duration>) -> f64 {
    duration.map_or(0.0, |duration| round_ms(duration.as_secs_f64() * 1_000.0))
}

/// Renders `timings_ms` from the durations the route measured once.
#[must_use]
pub(crate) fn timings(stages: &StageDurations) -> Value {
    let mut rendered = Map::new();
    for (key, duration) in TIMING_KEYS.iter().zip(stages.stages()) {
        rendered.insert((*key).to_owned(), json!(millis(duration)));
    }
    Value::Object(rendered)
}

/// Renders the closed `plan` object of one search (ADR-0119, decision 4).
///
/// `dense_strategy` and `rescore_candidates` are absent when no dense branch
/// ran; `alpha` appears only for `relative_score`.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    mode: Mode,
    strategy: Option<Strategy>,
    search: &SearchConfig,
    pool_size: u32,
    k: usize,
    exact: bool,
    grouping: &Grouping,
    fusion: FusionConfig,
) -> Value {
    let requested = candidates_per_branch(k, grouping);
    let mut rendered = Map::new();
    rendered.insert("mode".to_owned(), json!(mode.as_str()));
    if mode.uses_dense() {
        let dense = crible_search::dense::plan(search, pool_size, requested, exact);
        if let Some(strategy) = strategy {
            rendered.insert("dense_strategy".to_owned(), json!(strategy.as_str()));
        }
        rendered.insert(
            "rescore_candidates".to_owned(),
            json!(dense.rescore_candidates),
        );
    }
    rendered.insert("candidates_per_branch".to_owned(), json!(requested));
    rendered.insert("fusion".to_owned(), json!(fusion_name(fusion)));
    rendered.insert(
        "group_by".to_owned(),
        if grouping.by_document {
            json!("document")
        } else {
            Value::Null
        },
    );
    if let FusionConfig::RelativeScore { alpha } = fusion {
        rendered.insert("alpha".to_owned(), json!(alpha));
    }
    Value::Object(rendered)
}

/// Name of a fusion method in the plan (`docs/05-api.md`).
pub(crate) const fn fusion_name(fusion: FusionConfig) -> &'static str {
    match fusion {
        FusionConfig::Rrf => "rrf",
        FusionConfig::RelativeScore { .. } => "relative_score",
    }
}

/// Renders one `per_result` entry of the simple route.
#[must_use]
pub(crate) fn result_entry(hit: &RankedHit) -> Value {
    let mut rendered = Map::new();
    rendered.insert("key".to_owned(), json!(hit.hit.key));
    branch_members(&mut rendered, hit);
    rendered.insert("fused_rank".to_owned(), json!(hit.fused_rank));
    Value::Object(rendered)
}

/// Renders one `per_result` entry of the multi-index route.
#[must_use]
pub(crate) fn multi_result_entry(index: &str, hit: &RankedHit, target_rank: u32) -> Value {
    let mut rendered = Map::new();
    rendered.insert("index".to_owned(), json!(index));
    rendered.insert("key".to_owned(), json!(hit.hit.key));
    branch_members(&mut rendered, hit);
    rendered.insert("fused_rank".to_owned(), json!(hit.fused_rank));
    rendered.insert("target_rank".to_owned(), json!(target_rank));
    Value::Object(rendered)
}

/// Adds the branch members a result carries, omitting the absent ones.
fn branch_members(rendered: &mut Map<String, Value>, hit: &RankedHit) {
    if let Some(dense) = hit.dense {
        rendered.insert("dense_rank".to_owned(), json!(dense.rank));
        rendered.insert("dense_score".to_owned(), json!(f64::from(dense.score)));
    }
    if let Some(lexical) = hit.lexical {
        rendered.insert("lexical_rank".to_owned(), json!(lexical.rank));
        rendered.insert("lexical_score".to_owned(), json!(f64::from(lexical.score)));
    }
}

/// The simple-route part of `debug`, built on the blocking thread.
///
/// `plan` and `per_result` are ready there; `timings_ms` waits for the `total`
/// the route closes once, so the member and the analytics column carry the same
/// duration (ADR-0119, decision 7).
#[derive(Debug, Clone)]
pub(crate) struct SearchDebug {
    /// The closed `plan` object.
    pub(crate) plan: Value,
    /// One entry per result, in the order of `results`.
    pub(crate) per_result: Vec<Value>,
}

impl SearchDebug {
    /// Renders the whole `debug` member with the durations of the route.
    #[must_use]
    pub(crate) fn render(&self, stages: &StageDurations) -> Value {
        json!({
            "plan": self.plan,
            "timings_ms": timings(stages),
            "per_result": self.per_result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{TIMING_KEYS, multi_result_entry, result_entry, round_ms, timings};
    use crate::analytics::StageDurations;
    use crible_core::config::{FusionConfig, ScanStrategy, SearchConfig};
    use crible_search::group::Grouping;
    use crible_search::pipeline::{Mode, RankedHit};
    use serde_json::Value;
    use std::time::Duration;

    /// Dense settings of the tests.
    fn settings() -> SearchConfig {
        SearchConfig {
            scan: ScanStrategy::Binary,
            rescore_candidates: 5_000,
            exact_threshold: 1_000,
            candidates_per_branch: 100,
            fusion: FusionConfig::Rrf,
        }
    }

    /// A result with a payload, the members of its branches and its ranks.
    fn hit(dense: bool, lexical: bool, fused_rank: u32) -> RankedHit {
        RankedHit {
            hit: crible_search::dense::exact::Hit {
                doc_id: "D1".to_owned(),
                ord: 0,
                key: "D1#0".to_owned(),
                score: 0.5,
                fields: Value::Null,
                text: "text".to_owned(),
            },
            dense: dense.then_some(crible_search::fusion::BranchScore {
                rank: 70,
                score: 0.61,
            }),
            lexical: lexical.then_some(crible_search::fusion::BranchScore {
                rank: 2,
                score: 14.2,
            }),
            fused_rank,
        }
    }

    #[test]
    fn the_plan_is_closed_and_omits_the_dense_members_without_a_dense_branch() {
        // ADR-0119, decision 4: six members, plus `alpha` for `relative_score`.
        let plan = super::plan(
            Mode::Hybrid,
            Some(crible_search::dense::Strategy::Binary),
            &settings(),
            412_337,
            10,
            false,
            &Grouping::none(),
            FusionConfig::Rrf,
        );
        let object = plan.as_object().expect("the plan is an object");
        for key in [
            "mode",
            "dense_strategy",
            "rescore_candidates",
            "candidates_per_branch",
            "fusion",
            "group_by",
        ] {
            assert!(object.contains_key(key), "the plan carries {key}: {plan}");
        }
        assert_eq!(object.len(), 6, "the plan is closed: {plan}");
        assert_eq!(plan["mode"], "hybrid");
        assert_eq!(plan["dense_strategy"], "binary");
        assert_eq!(plan["rescore_candidates"], 5_000);
        assert_eq!(plan["candidates_per_branch"], 100);
        assert_eq!(plan["fusion"], "rrf");
        assert_eq!(plan["group_by"], Value::Null);

        // No dense branch: its two members are absent, never null.
        let lexical = super::plan(
            Mode::Lexical,
            None,
            &settings(),
            12,
            10,
            false,
            &Grouping::none(),
            FusionConfig::Rrf,
        );
        let object = lexical.as_object().expect("the plan is an object");
        assert!(!object.contains_key("dense_strategy"), "{lexical}");
        assert!(!object.contains_key("rescore_candidates"), "{lexical}");
        assert_eq!(object.len(), 4, "{lexical}");
    }

    #[test]
    fn the_plan_carries_alpha_only_for_relative_score() {
        let relative = super::plan(
            Mode::Hybrid,
            Some(crible_search::dense::Strategy::Exact),
            &settings(),
            12,
            10,
            false,
            &Grouping::none(),
            FusionConfig::RelativeScore { alpha: 0.3 },
        );
        assert_eq!(relative["fusion"], "relative_score");
        assert_eq!(relative["alpha"], 0.3);
    }

    #[test]
    fn the_timings_carry_the_nine_keys_and_zero_for_a_step_that_did_not_run() {
        // ADR-0119, decision 5: nine keys, three decimals, `0.0` when absent.
        let stages = StageDurations {
            validate: Some(Duration::from_micros(123)),
            encode: Some(Duration::from_millis(30)),
            ..StageDurations::default()
        };
        let timings = timings(&stages);
        let object = timings.as_object().expect("timings is an object");
        assert_eq!(object.len(), TIMING_KEYS.len(), "{timings}");
        for key in TIMING_KEYS {
            assert!(object.contains_key(key), "timings carries {key}: {timings}");
        }
        assert_eq!(timings["validate"], 0.123);
        assert_eq!(timings["encode"], 30.0);
        assert_eq!(timings["dense"], 0.0);
        assert_eq!(timings["total"], 0.0);
        assert_eq!(round_ms(1.234_56), 1.235);
    }

    #[test]
    fn a_branch_member_absent_is_omitted_never_null_or_zero() {
        // ADR-0119, decision 3.
        let dense_only = result_entry(&hit(true, false, 1));
        assert_eq!(dense_only["dense_rank"], 70);
        assert_eq!(dense_only["dense_score"], f64::from(0.61_f32));
        assert!(
            dense_only.get("lexical_rank").is_none(),
            "the lexical members are omitted: {dense_only}"
        );
        assert!(dense_only.get("lexical_score").is_none(), "{dense_only}");
        assert_eq!(dense_only["fused_rank"], 1);
        assert_eq!(dense_only["key"], "D1#0");

        let lexical_only = result_entry(&hit(false, true, 3));
        assert!(lexical_only.get("dense_rank").is_none(), "{lexical_only}");
        assert_eq!(lexical_only["lexical_rank"], 2);
        assert_eq!(lexical_only["fused_rank"], 3);

        let both = result_entry(&hit(true, true, 1));
        assert_eq!(both["dense_rank"], 70);
        assert_eq!(both["lexical_rank"], 2);
    }

    #[test]
    fn the_multi_entry_carries_its_index_and_target_rank() {
        let entry = multi_result_entry("articles", &hit(true, true, 1), 4);
        assert_eq!(entry["index"], "articles");
        assert_eq!(entry["target_rank"], 4);
        assert_eq!(entry["fused_rank"], 1);
        assert_eq!(entry["dense_rank"], 70);
    }
}
