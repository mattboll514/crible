// SPDX-License-Identifier: Apache-2.0

//! Attaching a validated batch to an index (`ATTACHER-LOT`).
//!
//! References: `docs/04-donnees-et-ingestion.md` ("Modes", "Sémantique
//! d'attache", "Le rapport d'attache"), `docs/02-architecture.md` ("Générations
//! et point de commit", "Tables SQLite", "Suppressions, pierres tombales et
//! compaction"), ADR-0016 (the SQLite commit is the only commit point),
//! ADR-0040 (unique `doc_id`s per role), ADR-0041 (removed document row,
//! manifest digest, six per-document counters), ADR-0052 (the acceptance
//! threshold is 0.999), ADR-0053 (the encoder of control 9 is loaded by the
//! caller) and ADR-0054 (the caller provides a loader; `attach` chooses when to
//! call it). `docs/08-invariants-et-tests.md` covers INV-03, INV-10, INV-11 and
//! INV-12.
//!
//! # Validation, segment, then one transaction
//!
//! An attach follows the order of `docs/02-architecture.md` ("Attacher un lot"):
//!
//! 1. [`BatchReader::open`] and [`BatchReader::validate`] check the batch, then
//!    control 8 counts the tokens of every passage and refuses an over-long one
//!    (**INV-11**), control 9 re-encodes a deterministic sample in fp32 and
//!    refuses vectors that do not match it (**INV-12**), and a read-only pass
//!    over `documents` decides the fate of every document. No state is modified
//!    yet.
//! 2. The segment is written in `segments/tmp-<id>/`, fsynced and renamed to
//!    `segments/<id>/`. It stays invisible until SQLite references it, so an
//!    attach that stops later leaves an unreferenced directory that recovery
//!    removes (INV-04).
//! 3. One SQLite transaction writes the document rows, the passage rows, the
//!    tombstones, the removed `documents` rows, the `batches` row, the
//!    `segments` row and `meta`. It goes through
//!    [`GenerationStore::commit_with_rows`](crate::generation::GenerationStore),
//!    which owns the commit point (ADR-0016). A failure before the commit rolls
//!    the transaction back, so the previous generation stays visible in full
//!    (INV-03).
//!
//! Once the commit point is passed, no SQLite write follows: the report stored
//! in `batches.report` is the one written by the transaction, with the six
//! counters, the acceptance statistics of control 9 and the generation. The
//! durations of the steps are call telemetry, returned to the caller that ran
//! the attach and never persisted (ADR-0042); a replay (INV-10) returns the
//! stored report, without durations.
//!
//! The batch files are streamed several times: validation, the read-only
//! planning pass, the segment write and the transaction each open them again.
//! Every pass keeps bounded record batches, and only the identifiers are held
//! in memory (the declared `doc_id`s, the ones to refresh and rewrite, and the
//! `iid`s to tombstone); texts, fields and vectors are streamed. The segment is
//! complete and renamed before the transaction opens, as
//! `docs/02-architecture.md` requires.
//!
//! # Semantics
//!
//! | Situation of the document | Effect | Counter |
//! |---|---|---|
//! | unknown `doc_id` | document and passages added | `added` |
//! | known, different `doc_hash` | old passages tombstoned, new ones added, row rewritten | `replaced` |
//! | known, same `doc_hash`, same fields | nothing written | `unchanged` |
//! | known, same `doc_hash`, other fields | row rewritten, no passage written | `metadata_updated` |
//! | known `doc_id` in `deletions`, or absent from a `replace_all` batch | passages tombstoned, row removed | `deleted` |
//! | unknown `doc_id` in `deletions` | nothing written | `ignored` |
//!
//! A document counts in exactly one category, so the six counters sum to the
//! number of documents the attach examined (ADR-0041). The document row of a
//! removed document is deleted in the commit transaction, which is what makes a
//! later re-addition a new document rather than a replay of a hash that no
//! longer has live passages (ADR-0041 decision 1).
//!
//! # Idempotence
//!
//! `manifest_blake3` is the blake3 digest of the bytes of `manifest.json` as
//! read, stored as `blake3:<hex>`. Applying an already-applied `batch_id` with
//! the same digest writes nothing and returns the report of the original attach
//! (INV-10); the same `batch_id` with another digest is a
//! [`ErrorCode::BatchConflict`] (HTTP 409). Both checks happen before the first
//! write.
//!
//! # Fault injection
//!
//! With the `failpoints` feature only, the attach stops at
//! `attach_validated` (step 1 of `docs/02`: validation, acceptance and the
//! semantic table done, nothing written), `after_attach_segment` (step 2: the
//! segment is renamed into `segments/<id>/`, no transaction open),
//! `after_attach_documents` (document rows written, passages not yet) and at
//! `after_attach_passages` (passage rows written, tombstones, `batches`, the
//! `segments` row and `meta` not yet; the segment files were already renamed
//! before the transaction). Steps 3, 4, 5 and 6 stop at their own points of the
//! commit protocol. Stopping means panicking with a message
//! containing `CRIBLE_FAILPOINT`, exactly as
//! [`GenerationStore::commit`](crate::generation::GenerationStore::commit)
//! does, so the transaction rolls back (INV-03).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arrow::array::{
    Array, BooleanArray, Date32Array, FixedSizeBinaryArray, Float64Array, Int64Array, RecordBatch,
    StringArray, UInt32Array,
};
use crible_core::batch::BatchMode;
use crible_core::batch::VectorDtype;
use crible_core::config::{
    FieldLevel, FieldType, IndexConfig, IngestConfig, ReferencePrecision, Schema,
};
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid};
use crible_core::model::{ModelConfig, ModelKind};
use crible_encode::{Model, Side, TokenCounter};
use half::f16;
use rusqlite::{Connection, Transaction};
use serde_json::{Map, Number, Value, json};

use crate::batch_reader::BatchReader;
use crate::filters_index::{CommitDelta, CreatedPassage};
use crate::generation::{GenerationStore, stop_at_failpoint};
use crate::records::{self, DocumentRecord, PassageRecord};
use crate::segment::{SegmentMeta, SegmentSpec, SegmentWriter};

/// What one attach changed, as `docs/04` asks.
///
/// Every counter counts **documents**, the unit of update: a document whose
/// `doc_hash` changed counts once under `replaced`, whatever its number of
/// passages.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachReport {
    /// Batch that was applied.
    pub batch_id: String,
    /// Generation the batch produced, or the generation of the original attach
    /// when the batch had already been applied (INV-10).
    pub generation: Generation,
    /// Documents the attach created.
    pub added: u64,
    /// Documents whose `doc_hash` differed.
    pub replaced: u64,
    /// Documents with the same `doc_hash` and the same fields.
    pub unchanged: u64,
    /// Documents with the same `doc_hash` but different fields.
    pub metadata_updated: u64,
    /// Documents removed by `deletions` or by `replace_all`.
    pub deleted: u64,
    /// Deletions naming a `doc_id` the index does not know, which are ignored.
    pub ignored: u64,
    /// Statistics of control 9 (the acceptance check of `docs/04`, INV-12).
    ///
    /// `None` means "no acceptance check ran": an `external` model has no
    /// encoder to re-encode with, and a batch without a passage has nothing to
    /// sample.
    pub acceptance: Option<AcceptanceReport>,
    /// Time spent in each step of this attach, when the report comes from the
    /// call that ran it.
    ///
    /// These durations are call telemetry: they are measured for the caller,
    /// never stored in `batches.report` (ADR-0042). A replay (INV-10) returns
    /// the stored report and therefore carries `None`, which means "not
    /// measured for this report", not "zero".
    pub durations: Option<AttachDurations>,
}

/// Statistics of the acceptance check of control 9, as `docs/05` names them.
///
/// They summarize the cosines between the vectors of the drawn sample and their
/// fp32 re-encoding: `sample` passages were re-encoded, and `min_cosine`, `p1`
/// and `p50` describe the distribution. They carry no identifier and no text
/// (AGENTS.md, "Confidentialité").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AcceptanceReport {
    /// Number of passages actually re-encoded: `acceptance_sample`, capped by
    /// the number of passages of the batch.
    pub sample: u32,
    /// Smallest cosine of the sample, the value compared to
    /// `acceptance_min_cosine`.
    pub min_cosine: f64,
    /// First percentile of the cosines, nearest rank.
    pub p1: f64,
    /// Median cosine, nearest rank.
    pub p50: f64,
}

impl AcceptanceReport {
    /// Serializes the statistics as the `acceptance` member of the report.
    fn to_json(self) -> Value {
        json!({
            "sample": self.sample,
            "min_cosine": self.min_cosine,
            "p1": self.p1,
            "p50": self.p50,
        })
    }

    /// Summarizes the cosines of one acceptance check.
    ///
    /// `sample` is the number of passages the check re-encoded and `cosines`
    /// holds one finite value per passage. The percentiles are nearest rank: no
    /// interpolation, so the reported value is always one that was measured.
    fn from_cosines(sample: usize, cosines: &mut [f64]) -> Result<Self, CribleError> {
        cosines.sort_by(f64::total_cmp);
        let min_cosine = cosines.first().copied().ok_or_else(no_cosine)?;
        let p1 = percentile(cosines, 1)?;
        let p50 = percentile(cosines, 50)?;
        let sample = u32::try_from(sample).map_err(|_| {
            CribleError::new(
                ErrorCode::Internal,
                format!("the acceptance sample of {sample} passages does not fit a u32"),
            )
        })?;
        Ok(Self {
            sample,
            min_cosine,
            p1,
            p50,
        })
    }
}

/// Time spent in each step of one attach, in the order of
/// `docs/02-architecture.md` ("Attacher un lot").
///
/// Validation includes the read-only pass that decides the fate of every
/// document: that pass is what lets the segment be written before the
/// transaction, and `docs/02` has no separate step for it.
///
/// The lexical step of `docs/02` does not exist yet (it belongs to phase 2).
/// The acceptance statistics of `docs/04` are not a duration of this structure:
/// they are content of the report, carried by
/// [`AttachReport::acceptance`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AttachDurations {
    /// Opening the batch, checking its controls, validating its content, and
    /// deciding the fate of every document with reads only (no write).
    pub validation: Duration,
    /// Writing, fsyncing and renaming the segment.
    pub segment: Duration,
    /// Filling the SQLite transaction and reaching the commit point.
    pub transaction: Duration,
    /// Writing `manifests/<generation>.json`.
    pub manifest: Duration,
    /// Building and publishing the new snapshot.
    pub snapshot: Duration,
}

impl AttachDurations {
    /// Truncates every duration to the microsecond, so the report of a call is
    /// comparable from one run to the next and small to log.
    fn to_microseconds(self) -> Self {
        Self {
            validation: Duration::from_micros(micros(self.validation)),
            segment: Duration::from_micros(micros(self.segment)),
            transaction: Duration::from_micros(micros(self.transaction)),
            manifest: Duration::from_micros(micros(self.manifest)),
            snapshot: Duration::from_micros(micros(self.snapshot)),
        }
    }
}

/// One duration in whole microseconds, as the report stores it.
fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

impl AttachReport {
    /// Serializes the report the `batches.report` column stores.
    ///
    /// The persisted report carries the six counters, the generation and the
    /// acceptance statistics of control 9. It never carries the `batch_id`,
    /// which the call already knows, nor durations (ADR-0042). The wire key of
    /// the ignored deletions is `ignored`, the name ADR-0041 and `docs/04` use.
    /// A missing acceptance check is written `null`, so a reader of the report
    /// tells "no check ran" from "a check ran and found nothing wrong".
    fn to_json(&self) -> Value {
        json!({
            "generation": self.generation.get(),
            "added": self.added,
            "replaced": self.replaced,
            "unchanged": self.unchanged,
            "metadata_updated": self.metadata_updated,
            "deleted": self.deleted,
            "ignored": self.ignored,
            "acceptance": self.acceptance.map(AcceptanceReport::to_json),
        })
    }

    /// Reads back the report stored by an earlier attach (INV-10).
    fn from_stored(report: &str, batch_id: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(report).map_err(|error| {
            CribleError::new(
                ErrorCode::Internal,
                format!("the stored report of batch '{batch_id}' is not valid JSON: {error}"),
            )
        })?;
        let generation = value
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| stored_report_error(batch_id, "generation"))?;
        Ok(Self {
            batch_id: batch_id.to_owned(),
            generation: Generation::new(generation),
            added: stored_count(&value, batch_id, "added")?,
            replaced: stored_count(&value, batch_id, "replaced")?,
            unchanged: stored_count(&value, batch_id, "unchanged")?,
            metadata_updated: stored_count(&value, batch_id, "metadata_updated")?,
            deleted: stored_count(&value, batch_id, "deleted")?,
            ignored: stored_count(&value, batch_id, "ignored")?,
            acceptance: stored_acceptance(&value, batch_id)?,
            durations: None,
        })
    }

    /// Builds the report of a live attach from its counters.
    fn from_counters(
        batch_id: &str,
        generation: Generation,
        counters: Counters,
        acceptance: Option<AcceptanceReport>,
    ) -> Self {
        Self {
            batch_id: batch_id.to_owned(),
            generation,
            added: counters.added,
            replaced: counters.replaced,
            unchanged: counters.unchanged,
            metadata_updated: counters.metadata_updated,
            deleted: counters.deleted,
            ignored: counters.ignored,
            acceptance,
            durations: None,
        }
    }
}

/// Counters accumulated while the transaction is being filled.
#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    added: u64,
    replaced: u64,
    unchanged: u64,
    metadata_updated: u64,
    deleted: u64,
    ignored: u64,
}

/// What the attach decided, before the passages are written.
#[derive(Debug, Default)]
struct Plan {
    /// Documents whose passages must be written: added and replaced ones.
    refresh: HashSet<String>,
    /// Documents whose `documents` row must be written: added, replaced and
    /// metadata-updated ones. An unchanged document writes nothing.
    rewrite: HashSet<String>,
    /// Documents the batch declares, used by `replace_all`.
    declared: HashSet<String>,
    /// Live passages captured before any new passage is added, so the
    /// tombstones kill the old rows only and never the replacement.
    tombstones: Vec<Iid>,
    /// Document rows to remove in the same transaction (ADR-0041).
    removed: Vec<String>,
    /// Documents already tombstoned, so a removal and a `replace_all` absent
    /// set cannot tombstone one document twice.
    retired: HashSet<String>,
    /// Documents already queued for row removal.
    removing: HashSet<String>,
}

impl Plan {
    /// Tombstones the live passages of one document: captured now, before any
    /// new passage is added, so the replacement kills the old rows only.
    fn retire_passages(
        &mut self,
        connection: &Connection,
        doc_id: &str,
    ) -> Result<(), CribleError> {
        if !self.retired.insert(doc_id.to_owned()) {
            return Ok(());
        }
        self.tombstones
            .extend(records::live_iids_of_document(connection, doc_id)?);
        Ok(())
    }

    /// Tombstones the live passages of one document **and** removes its row.
    fn remove_document(
        &mut self,
        connection: &Connection,
        doc_id: &str,
    ) -> Result<(), CribleError> {
        self.retire_passages(connection, doc_id)?;
        if self.removing.insert(doc_id.to_owned()) {
            self.removed.push(doc_id.to_owned());
        }
        Ok(())
    }
}

/// Validates `batch_dir` with [`BatchReader`], applies the semantic table of
/// `docs/04`, then commits one generation.
///
/// `index_config` and `model_config` are the configuration of the target index
/// and the configuration resolved for its model, exactly as
/// [`BatchReader::open`] wants them. The mode comes from `manifest.mode`, so
/// the batch declares how it applies.
///
/// `token_counter` is the tokenizer of the model, when the model is `onnx`: it
/// counts the tokens of every passage, prompt included, and refuses the batch
/// when one exceeds `max_tokens` (control 8 of `docs/04`, **INV-11**). An
/// `external` model has no tokenizer and is out of that control; an `onnx`
/// model without its counter is a caller fault, reported as
/// [`ErrorCode::Internal`].
///
/// `document_encoder` is how the caller loads the fp32 encoder of an `onnx`
/// model (control 9 of `docs/04`, **INV-12**). The caller provides a loader and
/// keeps the policy — which directory, which precision, which cache; `attach`
/// only chooses *when* to call it, after controls 3 and 8 have accepted the
/// batch and before the first write (ADR-0054). The loader returns an
/// [`Arc<Model>`](Model) so that the online mode of ADR-0055 hands out the very
/// graph it encoded with, instead of loading a second fp32 session for the
/// acceptance check. Loading it lazily is what spares
/// the ONNX Runtime session to a batch refused earlier, for another model or for
/// any other control. `None` is the `external` model, which has nothing to
/// re-encode with; an `onnx` model without a loader is a caller fault, reported
/// as [`ErrorCode::Internal`].
///
/// # Errors
///
/// - [`ErrorCode::BatchConflict`] when the same `batch_id` was applied with
///   another manifest digest;
/// - [`ErrorCode::InvalidRequest`] for a fault of the batch, including a
///   `doc_id` declared both in `documents` and in `deletions`, and a passage
///   longer than `max_tokens`;
/// - [`ErrorCode::IncompatibleModel`] when the batch announces another model;
/// - [`ErrorCode::AcceptanceFailed`] when the minimum cosine of the acceptance
///   sample is below `acceptance_min_cosine`;
/// - [`ErrorCode::Internal`] for an unexpected state of the store.
///
/// A replay of an applied batch is not an error: it writes nothing and returns
/// the report of the original attach, acceptance statistics included (INV-10).
///
/// **An error raised after the commit point.** The commit is the SQLite commit
/// of the generation (ADR-0016), and no SQLite write follows it: the report is
/// written in that transaction. Building the snapshot and writing the manifest
/// happen after it, so an error raised there returns `Err` while
/// `meta.generation = G+1` is already committed and the new generation is not
/// published. The next [`GenerationStore::open`] rewrites the manifest and
/// publishes that generation (INV-04). A caller must therefore not read such an
/// error as "nothing was written", exactly as
/// [`GenerationStore::commit`](crate::generation::GenerationStore::commit)
/// warns. A segment written before the transaction and never referenced is an
/// orphan that recovery removes.
pub fn attach(
    store: &mut GenerationStore,
    batch_dir: &Path,
    index_config: &IndexConfig,
    model_config: &ModelConfig,
    token_counter: Option<&TokenCounter>,
    document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>>,
) -> Result<AttachReport, CribleError> {
    let validation_started = Instant::now();
    let reader = BatchReader::open(batch_dir, index_config, model_config)?;
    let batch_id = reader.manifest().batch_id.clone();
    let manifest_blake3 = reader.manifest_blake3().to_owned();

    if let Some(applied) = store.applied_batch(&batch_id)? {
        if applied.manifest_blake3 == manifest_blake3 {
            return AttachReport::from_stored(&applied.report, &batch_id);
        }
        return Err(CribleError::new(
            ErrorCode::BatchConflict,
            format!(
                "batch '{batch_id}' was already applied with the manifest {0}, but this \
                 batch has {manifest_blake3}; replaying an applied batch_id requires the \
                 same bytes of manifest.json",
                applied.manifest_blake3
            ),
        ));
    }

    // Controls 6 and 7, including the cross-check of the two roles.
    reader.validate()?;
    // Control 8: no passage is truncated silently (INV-11).
    check_passage_lengths(&reader, model_config.kind, token_counter)?;
    // Control 9: the acceptance check re-encodes a deterministic sample in
    // fp32, and a refused batch never reaches the segment (INV-12).
    let acceptance = check_acceptance(
        &reader,
        model_config.kind,
        document_encoder,
        &index_config.ingest,
        index_config.reference_precision,
    )?;

    let mode = reader.manifest().mode;
    let manifest_json = reader.manifest_json().to_owned();
    let segments_dir = store.segments_directory();
    let segment_id = format!("seg-{batch_id}");
    let dims = model_config.dims;
    let precision = index_config.reference_precision;
    let schema = &index_config.schema;

    // The semantic table of `docs/04` is decided by reads only, before any
    // write: the segment and the transaction then apply that decision. It is
    // timed as part of the validation, which `docs/02` has no separate step for.
    let mut counters = Counters::default();
    let (plan, generation, next_iid) = store.with_connection(|connection| {
        let meta = records::read_meta(connection)?;
        let generation = crate::generation::generation_after(meta.generation)?;
        let plan = plan_documents(connection, &reader, schema, generation, mode, &mut counters)?;
        Ok((plan, generation, meta.next_iid))
    })?;
    let validation = validation_started.elapsed();
    // Step 1 of `docs/02`: validation, acceptance and the semantic table done,
    // no state modified.
    stop_at_failpoint("attach_validated");

    // Step 2 of `docs/02`: the segment is written and renamed before the
    // transaction opens.
    let segment_started = Instant::now();
    let segment = write_segment(
        &reader,
        &plan.refresh,
        &segments_dir,
        &SegmentTarget {
            segment_id: segment_id.clone(),
            iid_start: next_iid,
            generation,
            batch_id: batch_id.clone(),
            dims,
            precision,
        },
    )?;
    let segment_duration = segment_started.elapsed();
    // End of step 2 of `docs/02`: the segment is renamed into `segments/<id>/`
    // and no transaction is open. Until SQLite references it, recovery removes
    // it as an unreferenced segment directory (INV-03, INV-04).
    stop_at_failpoint("after_attach_segment");

    let committed =
        store.commit_with_rows(|transaction, committed_generation, committed_iid| {
            if committed_generation != generation {
                return Err(CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "the planned generation {} differs from the committed generation {}: \
                     another writer changed the index during the attach",
                        generation.get(),
                        committed_generation.get()
                    ),
                ));
            }
            if committed_iid != next_iid {
                return Err(CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "the planned next_iid {next_iid} differs from the committed next_iid \
                     {committed_iid}: another writer changed the index during the attach"
                    ),
                ));
            }

            write_document_rows(transaction, &reader, schema, &plan.rewrite, generation)?;
            stop_at_failpoint("after_attach_documents");

            let (after_passages, created) = write_passage_rows(
                transaction,
                &reader,
                schema,
                &plan.refresh,
                next_iid,
                &segment_id,
                generation,
            )?;
            let expected_next = match &segment {
                Some(segment) => u64::from(next_iid)
                    .checked_add(u64::from(segment.count))
                    .and_then(|next| u32::try_from(next).ok())
                    .ok_or_else(|| {
                        CribleError::new(
                            ErrorCode::Internal,
                            format!(
                                "segment {segment_id} of {} passages leaves no free iid",
                                segment.count
                            ),
                        )
                    })?,
                None => next_iid,
            };
            if after_passages != expected_next {
                return Err(CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "the attach wrote {} passages rows, but the segment holds {}: every row \
                     must have its vector",
                        u64::from(after_passages.saturating_sub(next_iid)),
                        segment.as_ref().map_or(0, |segment| segment.count)
                    ),
                ));
            }
            stop_at_failpoint("after_attach_passages");

            // Old passages die at G+1, and the row of a removed document goes with
            // them, in the same transaction (ADR-0041).
            for iid in &plan.tombstones {
                records::tombstone_passage(transaction, *iid, generation)?;
            }
            for doc_id in &plan.removed {
                records::delete_document(transaction, doc_id)?;
            }

            let applied = AttachReport::from_counters(&batch_id, generation, counters, acceptance);
            records::insert_batch(
                transaction,
                &applied.batch_id,
                generation,
                &manifest_blake3,
                &manifest_json,
                &report_json(&applied)?,
            )?;
            if let Some(segment) = &segment {
                records::insert_segment(transaction, segment)?;
            }
            Ok((
                after_passages,
                CommitDelta {
                    created,
                    rewritten_documents: plan.rewrite.iter().cloned().collect(),
                    tombstones: plan.tombstones.clone(),
                    removed_documents: plan.removed.clone(),
                },
            ))
        })?;

    let mut report =
        AttachReport::from_counters(&batch_id, committed.generation, counters, acceptance);
    report.durations = Some(
        AttachDurations {
            validation,
            segment: segment_duration,
            transaction: committed.timings.transaction,
            manifest: committed.timings.manifest,
            snapshot: committed.timings.snapshot,
        }
        .to_microseconds(),
    );
    debug_assert_eq!(report.generation, committed.generation);
    Ok(report)
}

/// Number of offending passage keys one length refusal lists.
///
/// The deliverable of `CONTROLER-LONGUEURS` names the first twenty keys, in the
/// order of the batch rows, so the message stays short on a batch that holds
/// thousands of over-long passages.
const LENGTH_REPORT_KEYS: usize = 20;

/// Control 8 of `docs/04-donnees-et-ingestion.md`: the token count of every
/// passage, prompt included, must not exceed `max_tokens` (**INV-11**).
///
/// The passages are streamed again in bounded record batches, so the memory
/// stays the one of [`BatchReader`] and never grows with the batch. Every
/// passage is counted, refreshed or not, because the batch is refused as a
/// whole. The count is parallel inside one record batch and keeps the row order,
/// so the reported keys are the first ones of the batch.
///
/// An `external` model declares neither tokenizer nor prompt: it is out of the
/// control, as `docs/04` requires. An `onnx` model without a counter is a caller
/// fault, never a silent acceptance.
fn check_passage_lengths(
    reader: &BatchReader,
    kind: ModelKind,
    counter: Option<&TokenCounter>,
) -> Result<(), CribleError> {
    let counter = match (kind, counter) {
        (ModelKind::External, _) => return Ok(()),
        (ModelKind::Onnx, Some(counter)) => counter,
        (ModelKind::Onnx, None) => {
            return Err(CribleError::new(
                ErrorCode::Internal,
                format!(
                    "batch '{}' belongs to an onnx model, whose passage lengths must be \
                     checked, but the caller passed no token counter (INV-11)",
                    reader.manifest().batch_id
                ),
            ));
        }
    };
    let max_tokens = counter.max_tokens();
    let limit = usize::try_from(max_tokens).unwrap_or(usize::MAX);
    let mut offending = 0_u64;
    let mut keys: Vec<String> = Vec::new();

    for chunk in reader.passages() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let ords = u32_column(&chunk, "ord")?;
        let texts = string_column(&chunk, "text")?;
        let mut values = Vec::with_capacity(chunk.num_rows());
        for row in 0..chunk.num_rows() {
            values.push(required_string(texts, row, "text")?);
        }
        for (row, count) in counter.count_documents(&values)?.into_iter().enumerate() {
            if count <= limit {
                continue;
            }
            offending = offending.saturating_add(1);
            if keys.len() < LENGTH_REPORT_KEYS {
                // The message carries keys only: never the text of a passage
                // (AGENTS.md, "Confidentialité").
                keys.push(format!(
                    "{}#{}",
                    required_string(ids, row, "doc_id")?,
                    ords.value(row)
                ));
            }
        }
    }

    if offending == 0 {
        return Ok(());
    }
    Err(invalid(format!(
        "batch '{}' is refused: {offending} passage(s) exceed max_tokens = {max_tokens} tokens, \
         prompt included (INV-11); the first {} keys, in the order of the batch rows, are: {}",
        reader.manifest().batch_id,
        keys.len(),
        keys.join(", ")
    )))
}

/// Control 9 of `docs/04-donnees-et-ingestion.md`: the acceptance check.
///
/// A deterministic sample of `acceptance_sample` passages, drawn with a seed
/// derived from the `batch_id`, is re-encoded in ONNX fp32 with the document
/// prompt, and each vector is compared with the one the batch stores. The batch
/// is refused with [`ErrorCode::AcceptanceFailed`] when the smallest cosine is
/// below `acceptance_min_cosine` (**INV-12**); otherwise the statistics of the
/// sample go to the report.
///
/// The passages are streamed in bounded record batches and only the drawn ones
/// are kept — their text and their stored vector, never the whole batch
/// (ADR-0039). The error message carries figures only: never the text of a
/// passage (AGENTS.md, "Confidentialité").
///
/// An `external` model has no encoder to re-encode with and is out of the
/// control, as `docs/04` requires. A batch without a passage has nothing to
/// sample: it carries no statistics rather than a fabricated distribution. A
/// sample size of zero cannot reach this function, since `crible-core` refuses
/// it at index creation and at `PATCH` (ADR-0054).
fn check_acceptance(
    reader: &BatchReader,
    kind: ModelKind,
    document_encoder: Option<&dyn Fn() -> Result<Arc<Model>, CribleError>>,
    ingest: &IngestConfig,
    precision: ReferencePrecision,
) -> Result<Option<AcceptanceReport>, CribleError> {
    if kind == ModelKind::External {
        return Ok(None);
    }
    let batch_id = reader.manifest().batch_id.clone();
    let document_encoder = document_encoder.ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "batch '{batch_id}' belongs to an onnx model, whose vectors must pass the \
                 acceptance check, but the caller passed no fp32 encoder (INV-12)"
            ),
        )
    })?;
    let wanted =
        acceptance_sample_size(ingest.acceptance_sample, reader.manifest().counts.passages);
    if wanted == 0 {
        // `crible-core` refuses a configuration with `acceptance_sample = 0`
        // (ADR-0054), so an empty sample here means the batch holds no passage
        // at all, a deletion-only batch: there is nothing to draw.
        return Ok(None);
    }

    let drawn = draw_acceptance_sample(reader, &batch_id, wanted, precision)?;
    let encoder = document_encoder()?;
    let mut cosines = Vec::with_capacity(drawn.len());
    for passage in &drawn {
        let reencoded = encoder.encode(Side::Document, &passage.text)?;
        cosines.push(cosine(&passage.vector, &reencoded, &batch_id)?);
    }
    let report = AcceptanceReport::from_cosines(drawn.len(), &mut cosines)?;
    if report.min_cosine < ingest.acceptance_min_cosine {
        return Err(CribleError::new(
            ErrorCode::AcceptanceFailed,
            format!(
                "batch '{batch_id}' is refused: the acceptance check re-encoded {} passage(s) in \
                 fp32 and got min cosine {:.6}, below acceptance_min_cosine = {:.6} (INV-12); p1 = \
                 {:.6}, p50 = {:.6}",
                report.sample,
                report.min_cosine,
                ingest.acceptance_min_cosine,
                report.p1,
                report.p50
            ),
        ));
    }
    Ok(Some(report))
}

/// Number of passages the acceptance check re-encodes: `acceptance_sample`,
/// capped by the passages of the batch.
fn acceptance_sample_size(configured: u32, passages: u64) -> usize {
    usize::try_from(u64::from(configured).min(passages)).unwrap_or(usize::MAX)
}

/// One passage kept by the acceptance draw, with the vector the batch stores.
struct SampledPassage {
    /// Priority in the draw; the smallest priorities are kept.
    priority: u64,
    /// Position of the row in the batch, from zero.
    index: u64,
    /// Text of the passage, which the check re-encodes with the document
    /// prompt.
    text: String,
    /// Vector of the passage, decoded from the batch into fp32.
    vector: Vec<f32>,
}

impl Ord for SampledPassage {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.priority, self.index).cmp(&(other.priority, other.index))
    }
}

impl PartialOrd for SampledPassage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SampledPassage {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SampledPassage {}

/// Draws the acceptance sample: the `wanted` passages of smallest priority,
/// where a priority is a blake3 digest of the `batch_id` digest and the row
/// index.
///
/// The draw is a pure function of `(batch_id, wanted, rows)`, so two attaches of
/// one batch re-encode the same passages and report the same distribution. It
/// holds at most `wanted` passages, whatever the size of the batch (ADR-0039).
fn draw_acceptance_sample(
    reader: &BatchReader,
    batch_id: &str,
    wanted: usize,
    precision: ReferencePrecision,
) -> Result<Vec<SampledPassage>, CribleError> {
    let seed = blake3::hash(batch_id.as_bytes());
    let mut sample: BinaryHeap<SampledPassage> = BinaryHeap::with_capacity(wanted);
    let mut index = 0_u64;

    for chunk in reader.passages() {
        let chunk = chunk?;
        let texts = string_column(&chunk, "text")?;
        let vectors = fixed_binary_column(&chunk, "vector")?;
        for row in 0..chunk.num_rows() {
            let priority = acceptance_priority(seed.as_bytes(), index);
            let position = index;
            index = index.saturating_add(1);
            if let Some(worst) = sample.peek()
                && sample.len() >= wanted
                && (priority, position) >= (worst.priority, worst.index)
            {
                continue;
            }
            let text = required_string(texts, row, "text")?;
            let vector = decode_vector(vectors.value(row), precision)?;
            if sample.len() == wanted {
                sample.pop();
            }
            sample.push(SampledPassage {
                priority,
                index: position,
                text: text.to_owned(),
                vector,
            });
        }
    }

    let mut drawn = sample.into_vec();
    drawn.sort_by_key(|passage| passage.index);
    Ok(drawn)
}

/// Priority of one batch row in the acceptance draw.
fn acceptance_priority(seed: &[u8; 32], index: u64) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed);
    hasher.update(&index.to_le_bytes());
    let digest = hasher.finalize();
    let mut head = [0_u8; 8];
    head.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(head)
}

/// Cosine of two vectors of the same width, computed in `f64`.
///
/// A cosine that is not a number comes from a zero-norm vector, which no
/// comparison can rank: the batch is refused as a fault of its content. The
/// result is clamped to `[-1, 1]`, where rounding may overshoot.
fn cosine(left: &[f32], right: &[f32], batch_id: &str) -> Result<f64, CribleError> {
    if left.len() != right.len() {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "the acceptance check compares vectors of {} and {} components in batch \
                 '{batch_id}'",
                left.len(),
                right.len()
            ),
        ));
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (left, right) in left.iter().zip(right) {
        dot += f64::from(*left) * f64::from(*right);
        left_norm += f64::from(*left) * f64::from(*left);
        right_norm += f64::from(*right) * f64::from(*right);
    }
    let cosine = dot / (left_norm.sqrt() * right_norm.sqrt());
    if !cosine.is_finite() {
        return Err(invalid(format!(
            "batch '{batch_id}' is refused: a sampled vector has no cosine with its re-encoding \
             (zero norm)"
        )));
    }
    Ok(cosine.clamp(-1.0, 1.0))
}

/// Nearest-rank percentile of a sorted slice: the value of rank
/// `ceil(percent × len / 100)`, clamped to the slice.
fn percentile(sorted: &[f64], percent: u32) -> Result<f64, CribleError> {
    let len = sorted.len();
    if len == 0 {
        return Err(no_cosine());
    }
    let scaled = u64::from(percent) * u64::try_from(len).unwrap_or(u64::MAX);
    let rank = usize::try_from(scaled.div_ceil(100)).unwrap_or(len).max(1);
    sorted.get(rank.min(len) - 1).copied().ok_or_else(no_cosine)
}

/// Error raised when an acceptance check has no cosine to summarize.
fn no_cosine() -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        "the acceptance check has no cosine to summarize",
    )
}

/// Reads the batch documents and the reference state, and decides what happens
/// to every document of the index. **No write happens here**: the transaction
/// applies this plan, which is what lets the segment be written before it.
///
/// `generation` is the generation the attach will commit (G+1); the document
/// rows and the segment it stamps are written later, with that value.
fn plan_documents(
    connection: &Connection,
    reader: &BatchReader,
    schema: &Schema,
    generation: Generation,
    mode: BatchMode,
    counters: &mut Counters,
) -> Result<Plan, CribleError> {
    let mut plan = Plan::default();

    for chunk in reader.documents() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let hashes = string_column(&chunk, "doc_hash")?;
        for row in 0..chunk.num_rows() {
            let doc_id = required_string(ids, row, "doc_id")?;
            let doc_hash = required_string(hashes, row, "doc_hash")?;
            let fields = fields_value(&chunk, row, schema, FieldLevel::Document)?;
            let record = DocumentRecord {
                doc_id: doc_id.to_owned(),
                doc_hash: doc_hash.to_owned(),
                fields,
                updated_generation: generation,
            };
            plan.declared.insert(doc_id.to_owned());

            match records::document_by_id(connection, doc_id)? {
                None => {
                    counters.added = counters.added.saturating_add(1);
                    plan.rewrite.insert(doc_id.to_owned());
                    plan.refresh.insert(doc_id.to_owned());
                }
                Some(previous) if previous.doc_hash != record.doc_hash => {
                    counters.replaced = counters.replaced.saturating_add(1);
                    plan.retire_passages(connection, doc_id)?;
                    plan.rewrite.insert(doc_id.to_owned());
                    plan.refresh.insert(doc_id.to_owned());
                }
                Some(previous) if previous.fields != record.fields => {
                    counters.metadata_updated = counters.metadata_updated.saturating_add(1);
                    plan.rewrite.insert(doc_id.to_owned());
                }
                Some(_) => {
                    counters.unchanged = counters.unchanged.saturating_add(1);
                }
            }
        }
    }

    for chunk in reader.deletions() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        for row in 0..chunk.num_rows() {
            let doc_id = required_string(ids, row, "doc_id")?;
            if records::document_by_id(connection, doc_id)?.is_some() {
                counters.deleted = counters.deleted.saturating_add(1);
                plan.remove_document(connection, doc_id)?;
            } else {
                counters.ignored = counters.ignored.saturating_add(1);
            }
        }
    }

    if mode == BatchMode::ReplaceAll {
        for doc_id in records::document_ids(connection)? {
            if plan.declared.contains(&doc_id) {
                continue;
            }
            counters.deleted = counters.deleted.saturating_add(1);
            plan.remove_document(connection, &doc_id)?;
        }
    }

    Ok(plan)
}

/// Streams the document files and writes the `documents` rows of the planned
/// documents, inside the transaction of the generation.
///
/// The files are read again, rather than keeping one row per document in
/// memory: the set of identifiers is what the plan holds, not their fields.
fn write_document_rows(
    transaction: &Transaction<'_>,
    reader: &BatchReader,
    schema: &Schema,
    rewrite: &HashSet<String>,
    generation: Generation,
) -> Result<(), CribleError> {
    for chunk in reader.documents() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let hashes = string_column(&chunk, "doc_hash")?;
        for row in 0..chunk.num_rows() {
            let doc_id = required_string(ids, row, "doc_id")?;
            if !rewrite.contains(doc_id) {
                continue;
            }
            let doc_hash = required_string(hashes, row, "doc_hash")?;
            records::insert_document(
                transaction,
                &DocumentRecord {
                    doc_id: doc_id.to_owned(),
                    doc_hash: doc_hash.to_owned(),
                    fields: fields_value(&chunk, row, schema, FieldLevel::Document)?,
                    updated_generation: generation,
                },
            )?;
        }
    }
    Ok(())
}

/// Identity and layout shared by every passage vector of one attach's segment.
///
/// The target is fixed for the whole attach: it names the segment, its place in
/// the epoch (`generation`, `iid_start`, `batch_id`) and the shape of the
/// vectors it stores (`dims`, `precision`).
struct SegmentTarget {
    /// Identifier of the segment file, derived from the batch id.
    segment_id: String,
    /// First free internal identifier of the generation.
    iid_start: u32,
    /// Generation that makes the segment visible.
    generation: Generation,
    /// Batch that produced the segment.
    batch_id: String,
    /// Number of dimensions of each vector.
    dims: u32,
    /// Storage precision of the vectors.
    precision: ReferencePrecision,
}

/// Streams the passage files, writes the vector of every refreshed document
/// into a new segment, and returns the first free `iid` after them.
///
/// A document that is unchanged or only metadata-updated keeps its live
/// passages, so its rows are skipped: only the documents of [`Plan::refresh`]
/// reach the segment. When no passage is written, no segment is created, which
/// is what a metadata-only or deletion-only attach expects.
fn write_segment(
    reader: &BatchReader,
    refresh: &HashSet<String>,
    segments_dir: &Path,
    target: &SegmentTarget,
) -> Result<Option<SegmentMeta>, CribleError> {
    let mut writer: Option<SegmentWriter> = None;

    for chunk in reader.passages() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let vectors = fixed_binary_column(&chunk, "vector")?;
        for row in 0..chunk.num_rows() {
            let doc_id = required_string(ids, row, "doc_id")?;
            if !refresh.contains(doc_id) {
                continue;
            }
            let vector = decode_vector(vectors.value(row), target.precision)?;

            if writer.is_none() {
                writer = Some(SegmentWriter::create(
                    segments_dir,
                    SegmentSpec {
                        segment_id: target.segment_id.clone(),
                        iid_start: Iid::new(target.iid_start),
                        dims: target.dims,
                        generation: target.generation,
                        batch_id: target.batch_id.clone(),
                        precision: target.precision,
                    },
                )?);
            }
            let writer = writer.as_mut().ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the segment writer must exist once a passage is written",
                )
            })?;
            writer.append(&vector)?;
        }
    }

    writer.map(SegmentWriter::finish).transpose()
}

/// Streams the passage files and writes the `passages` rows of the refreshed
/// documents, in the same order as [`write_segment`], so each row takes the
/// `iid` of its vector. Returns the first free `iid` after them, and the
/// passages it created, for the delta of
/// [`crate::filters_index::FilterIndex::after_commit`] (ADR-0066).
fn write_passage_rows(
    transaction: &Transaction<'_>,
    reader: &BatchReader,
    schema: &Schema,
    refresh: &HashSet<String>,
    iid_start: u32,
    segment_id: &str,
    generation: Generation,
) -> Result<(u32, Vec<CreatedPassage>), CribleError> {
    let mut next_iid = iid_start;
    let mut created = Vec::new();
    // One compressor for the whole batch of passage rows (ADR-0048).
    let mut compressor = records::TextCompressor::new()?;

    for chunk in reader.passages() {
        let chunk = chunk?;
        let ids = string_column(&chunk, "doc_id")?;
        let ords = u32_column(&chunk, "ord")?;
        let texts = string_column(&chunk, "text")?;
        for row in 0..chunk.num_rows() {
            let doc_id = required_string(ids, row, "doc_id")?;
            if !refresh.contains(doc_id) {
                continue;
            }
            let iid = Iid::new(next_iid);
            next_iid = next_iid.checked_add(1).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!("iid {} leaves no free identifier after it", iid.get()),
                )
            })?;
            let text = required_string(texts, row, "text")?;
            let record = PassageRecord {
                iid,
                doc_id: doc_id.to_owned(),
                ord: ords.value(row),
                text: text.to_owned(),
                passage_hash: passage_hash(text),
                fields: fields_value(&chunk, row, schema, FieldLevel::Passage)?,
                segment_id: segment_id.to_owned(),
                created_generation: generation,
                dead_since_gen: None,
            };
            records::insert_passage(transaction, &record, &mut compressor)?;
            created.push(CreatedPassage::from(record));
        }
    }

    Ok((next_iid, created))
}

/// Builds the JSON object of one row's fields for `level`, as canonical JSON.
///
/// Every declared field of that level is present, with `null` when the Parquet
/// column is null: ADR-0035 relies on the JSON form to tell a null from an
/// absent field. A `date` is written `YYYY-MM-DD`, the form its type documents.
fn fields_value(
    chunk: &RecordBatch,
    row: usize,
    schema: &Schema,
    level: FieldLevel,
) -> Result<Value, CribleError> {
    let mut fields = Map::new();
    for (name, field_level, spec) in schema.fields() {
        if field_level != level {
            continue;
        }
        let column = chunk.column_by_name(name).ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("a checked Parquet batch has no column '{name}'"),
            )
        })?;
        fields.insert(
            name.to_owned(),
            field_value(column.as_ref(), row, spec.field_type, name)?,
        );
    }
    Ok(Value::Object(fields))
}

/// Reads one field of one row and converts it to its canonical JSON value.
fn field_value(
    column: &dyn Array,
    row: usize,
    field_type: FieldType,
    name: &str,
) -> Result<Value, CribleError> {
    if column.is_null(row) {
        return Ok(Value::Null);
    }
    match field_type {
        FieldType::Keyword => Ok(Value::String(
            string_array(column, name)?.value(row).to_owned(),
        )),
        FieldType::Integer => Ok(Value::from(int64_array(column, name)?.value(row))),
        FieldType::Float => float_value(f64_array(column, name)?.value(row), name),
        FieldType::Date => Ok(Value::String(format_date(
            date32_array(column, name)?.value(row),
        ))),
        FieldType::Bool => Ok(Value::Bool(bool_array(column, name)?.value(row))),
    }
}

/// Converts one `float` field, refusing a value JSON cannot carry.
fn float_value(value: f64, name: &str) -> Result<Value, CribleError> {
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| invalid(format!("field '{name}' is not a finite number ({value})")))
}

/// `YYYY-MM-DD` of a count of days since 1970-01-01, as `date32` stores it.
fn format_date(days: i32) -> String {
    let (year, month, day) = civil_from_days(i64::from(days));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Converts a count of days since 1970-01-01 to a civil date.
///
/// Howard Hinnant's `civil_from_days`, which is exact over the whole `i32` range
/// of a `date32` and never depends on a platform clock or time zone.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

/// Digest of a passage text, which `dedup: content` compares.
fn passage_hash(text: &str) -> String {
    format!("blake3:{}", blake3::hash(text.as_bytes()).to_hex())
}

/// Decodes one stored vector into the components the segment writer appends.
fn decode_vector(bytes: &[u8], precision: ReferencePrecision) -> Result<Vec<f32>, CribleError> {
    let width = match precision {
        ReferencePrecision::Fp16 => 2,
        ReferencePrecision::Fp32 => 4,
    };
    if !bytes.len().is_multiple_of(width) {
        return Err(CribleError::new(
            ErrorCode::Internal,
            format!(
                "a stored {} vector is {} bytes, which is not a multiple of {width}",
                precision_name(precision),
                bytes.len()
            ),
        ));
    }
    bytes
        .chunks_exact(width)
        .map(|component| match precision {
            ReferencePrecision::Fp16 => {
                let raw: [u8; 2] = component.try_into().map_err(|_| {
                    CribleError::new(ErrorCode::Internal, "an fp16 component is not 2 bytes")
                })?;
                Ok(f32::from(f16::from_bits(u16::from_le_bytes(raw))))
            }
            ReferencePrecision::Fp32 => {
                let raw: [u8; 4] = component.try_into().map_err(|_| {
                    CribleError::new(ErrorCode::Internal, "an fp32 component is not 4 bytes")
                })?;
                Ok(f32::from_le_bytes(raw))
            }
        })
        .collect()
}

/// Wire name of a precision for error messages.
///
/// The single definition of the wire name is [`VectorDtype::as_str`], which the
/// batch manifest already writes: the store does not repeat the mapping.
fn precision_name(precision: ReferencePrecision) -> &'static str {
    VectorDtype::from_reference_precision(precision).as_str()
}

/// Borrows one required non-null string value.
fn required_string<'a>(
    column: &'a StringArray,
    row: usize,
    name: &str,
) -> Result<&'a str, CribleError> {
    if column.is_null(row) {
        return Err(invalid(format!(
            "column '{name}' of a validated batch holds null"
        )));
    }
    Ok(column.value(row))
}

/// Borrows a UTF-8 column of a checked record batch.
fn string_column<'a>(chunk: &'a RecordBatch, name: &str) -> Result<&'a StringArray, CribleError> {
    downcast(chunk.column_by_name(name).map(AsRef::as_ref), name)
}

/// Borrows a `UInt32` column of a checked record batch.
fn u32_column<'a>(chunk: &'a RecordBatch, name: &str) -> Result<&'a UInt32Array, CribleError> {
    downcast(chunk.column_by_name(name).map(AsRef::as_ref), name)
}

/// Borrows a fixed-size binary column of a checked record batch.
fn fixed_binary_column<'a>(
    chunk: &'a RecordBatch,
    name: &str,
) -> Result<&'a FixedSizeBinaryArray, CribleError> {
    downcast(chunk.column_by_name(name).map(AsRef::as_ref), name)
}

/// Borrows a string Arrow array by name.
fn string_array<'a>(column: &'a dyn Array, name: &str) -> Result<&'a StringArray, CribleError> {
    downcast(Some(column), name)
}

/// Borrows an `Int64` Arrow array by name.
fn int64_array<'a>(column: &'a dyn Array, name: &str) -> Result<&'a Int64Array, CribleError> {
    downcast(Some(column), name)
}

/// Borrows a `Float64` Arrow array by name.
fn f64_array<'a>(column: &'a dyn Array, name: &str) -> Result<&'a Float64Array, CribleError> {
    downcast(Some(column), name)
}

/// Borrows a `Date32` Arrow array by name.
fn date32_array<'a>(column: &'a dyn Array, name: &str) -> Result<&'a Date32Array, CribleError> {
    downcast(Some(column), name)
}

/// Borrows a boolean Arrow array by name.
fn bool_array<'a>(column: &'a dyn Array, name: &str) -> Result<&'a BooleanArray, CribleError> {
    downcast(Some(column), name)
}

/// Downcasts one column of a batch whose schema was already checked.
fn downcast<'a, T: 'static>(
    column: Option<&'a dyn Array>,
    name: &str,
) -> Result<&'a T, CribleError> {
    column
        .and_then(|column| column.as_any().downcast_ref::<T>())
        .ok_or_else(|| {
            CribleError::new(
                ErrorCode::Internal,
                format!("column '{name}' of a checked Parquet batch has an unexpected type"),
            )
        })
}

/// A fault of the batch.
fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Serializes the report stored in `batches.report`.
fn report_json(report: &AttachReport) -> Result<String, CribleError> {
    serde_json::to_string(&report.to_json()).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!("cannot serialize the report of an attach: {error}"),
        )
    })
}

/// Reads one counter of a stored report.
fn stored_count(value: &Value, batch_id: &str, key: &str) -> Result<u64, CribleError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| stored_report_error(batch_id, key))
}

/// Reads the acceptance statistics of a stored report (INV-10).
///
/// A report written before control 9 existed, an `external` model and a batch
/// without a passage carry no statistics: an absent member and an explicit
/// `null` both mean "no acceptance check ran".
fn stored_acceptance(
    value: &Value,
    batch_id: &str,
) -> Result<Option<AcceptanceReport>, CribleError> {
    let acceptance = match value.get("acceptance") {
        None | Some(Value::Null) => return Ok(None),
        Some(acceptance) => acceptance,
    };
    let sample = acceptance
        .get("sample")
        .and_then(Value::as_u64)
        .and_then(|sample| u32::try_from(sample).ok())
        .ok_or_else(|| stored_report_error(batch_id, "acceptance.sample"))?;
    Ok(Some(AcceptanceReport {
        sample,
        min_cosine: stored_statistic(acceptance, batch_id, "min_cosine")?,
        p1: stored_statistic(acceptance, batch_id, "p1")?,
        p50: stored_statistic(acceptance, batch_id, "p50")?,
    }))
}

/// Reads one numeric member of the stored acceptance statistics.
fn stored_statistic(acceptance: &Value, batch_id: &str, key: &str) -> Result<f64, CribleError> {
    acceptance
        .get(key)
        .and_then(Value::as_f64)
        .filter(|statistic| statistic.is_finite())
        .ok_or_else(|| stored_report_error(batch_id, &format!("acceptance.{key}")))
}

/// Error naming the missing member of a stored report.
fn stored_report_error(batch_id: &str, key: &str) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!("the stored report of batch '{batch_id}' has no '{key}' member"),
    )
}

#[cfg(test)]
mod tests {
    use crible_core::ids::Generation;

    use super::{
        AcceptanceReport, AttachReport, Counters, acceptance_sample_size, civil_from_days,
        format_date, percentile,
    };

    #[test]
    fn a_date32_converts_to_the_calendar_day() {
        for (days, expected) in [
            (0, "1970-01-01"),
            (1, "1970-01-02"),
            (-1, "1969-12-31"),
            (18_321, "2020-02-29"),
            (19_000, "2022-01-08"),
        ] {
            assert_eq!(format_date(days), expected, "date32 day {days}");
        }
    }

    #[test]
    fn civil_from_days_round_trips_a_leap_day() {
        assert_eq!(civil_from_days(18_321), (2020, 2, 29));
    }

    #[test]
    fn the_acceptance_sample_is_capped_by_the_passages_of_the_batch() {
        assert_eq!(acceptance_sample_size(200, 7_628), 200);
        assert_eq!(acceptance_sample_size(4, 6), 4);
        assert_eq!(acceptance_sample_size(500, 10), 10);
        assert_eq!(acceptance_sample_size(0, 10), 0);
    }

    #[test]
    fn the_stored_report_round_trips_the_acceptance_statistics() {
        let acceptance = AcceptanceReport {
            sample: 4,
            min_cosine: 0.9995,
            p1: 0.9997,
            p50: 0.9999,
        };
        let live = AttachReport::from_counters(
            "recette",
            Generation::new(2),
            Counters::default(),
            Some(acceptance),
        );
        let stored = super::report_json(&live);
        assert!(stored.is_ok(), "the report must serialize: {stored:?}");
        let Ok(stored) = stored else {
            return;
        };
        let read = AttachReport::from_stored(&stored, "recette");
        assert!(read.is_ok(), "the stored report must be readable: {read:?}");
        let Ok(read) = read else {
            return;
        };
        assert_eq!(read.acceptance, Some(acceptance));
        assert_eq!(read.durations, None);
        assert_eq!(read.generation, live.generation);
    }

    #[test]
    fn a_report_without_acceptance_round_trips_as_an_explicit_null() {
        let live =
            AttachReport::from_counters("externe", Generation::new(1), Counters::default(), None);
        let stored = super::report_json(&live);
        assert!(stored.is_ok(), "the report must serialize: {stored:?}");
        let Ok(stored) = stored else {
            return;
        };
        assert!(
            stored.contains("\"acceptance\":null"),
            "a report without an acceptance check must say so: {stored}"
        );
        let read = AttachReport::from_stored(&stored, "externe");
        assert!(read.is_ok(), "the stored report must be readable: {read:?}");
        let Ok(read) = read else {
            return;
        };
        assert_eq!(read.acceptance, None);
    }

    #[test]
    fn the_percentiles_are_nearest_rank() {
        let cosines: Vec<f64> = (1..=100).map(|value| f64::from(value) / 100.0).collect();
        let p1 = percentile(&cosines, 1);
        let p50 = percentile(&cosines, 50);
        let p100 = percentile(&cosines, 100);
        assert!(p1.is_ok() && p50.is_ok() && p100.is_ok());
        assert_eq!(
            (p1.ok(), p50.ok(), p100.ok()),
            (Some(0.01), Some(0.50), Some(1.0)),
            "nearest rank over a hundred cosines"
        );
    }

    #[test]
    fn a_percentile_of_an_empty_sample_is_an_error() {
        assert!(percentile(&[], 50).is_err());
    }
}
