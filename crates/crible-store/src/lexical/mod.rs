// SPDX-License-Identifier: Apache-2.0

//! Lexical branch of an index: the tantivy side of the storage.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale (tantivy)"),
//! `docs/02-architecture.md` ("Organisation sur disque", `epochs/<epoch>/lexical/`,
//! and "Générations et point de commit", step 4, "Reprise au démarrage"),
//! `docs/07-stack-et-deploiement.md` (tantivy 0.27), ADR-0007 (tantivy for
//! BM25), ADR-0016 (the SQLite commit is the only commit point), ADR-0086
//! (`lexical.language` and the analysis chain), ADR-0087 (the recovery runs on
//! a write open only) and ADR-0088 (an index is up to date when `M ≤ Gt ≤ G`,
//! and an unreadable state is rebuilt on a write open).
//!
//! # Layout
//!
//! One tantivy index per epoch, at `<index>/epochs/<epoch>/lexical/`. It is a
//! **derived** directory: it is rebuilt from `records.sqlite`, never a source of
//! truth, and the SQLite commit of a generation is the only commit point
//! (ADR-0016).
//!
//! # Documents
//!
//! One tantivy document per passage, with two fields (`docs/03`):
//!
//! | Field | Type | Options |
//! |---|---|---|
//! | `iid` | `u64` | indexed and fast |
//! | `text` | text | analysed by [`analyzer`] under the tokenizer name `TOKENIZER_NAME`, not stored |
//!
//! The indexed text is the encoder input, exactly the text of the dense branch:
//! it is the `passages.text` of `records.sqlite`, which the commit hands over as
//! it wrote it. Term positions are stored only when `lexical.positions` asks for
//! them; `docs/03` leaves them off by default.
//!
//! # The payload of a commit
//!
//! Every commit carries the generation whose text it indexed as its payload, as
//! the decimal ASCII number of [`Generation`] — the same form `CURRENT_EPOCH`
//! uses, readable with `cat` inside tantivy's `meta.json`. It is an internal
//! marker of a derived directory, never a public format.
//!
//! The payload alone does not say whether an epoch's lexical index is up to
//! date: a generation that changes no passage text — a metadata patch — produces
//! no commit, so the payload legitimately stays behind (`docs/02`, step 4). The
//! index is up to date for the committed generation `G` when `M ≤ Gt ≤ G`, where
//! `Gt` is the payload of its last commit and `M` is
//! `meta.passage_change_generation`, the last generation that created or killed
//! a passage (`ADR-0088`, decision 2). `observe` is the one place that reads
//! that state; the restart turns it into a repair on a **write** open, and a
//! read-only open only publishes what it observes (ADR-0087).
//!
//! # Writer
//!
//! `writer` opens the index of the epoch, or creates it, and returns a
//! `LexicalWriter` whose `LexicalWriter::commit` ends one tantivy commit. The
//! step 4 of `docs/02` builds one writer per generation: one writer thread and a
//! fixed memory budget (`WRITER_MEMORY_BUDGET_IN_BYTES`), which is a constant
//! of the code, not a setting (`AGENTS.md` rule 12).
//!
//! # Configuration changes
//!
//! This module indexes with the configuration the epoch's `index.json` declares.
//! A change of `lexical.language` or `lexical.positions` on an epoch that already
//! has a lexical index requires the rebuild job of `RECONSTRUIRE-LEXICAL`; this
//! task does not detect that change.

pub mod analyzer;

use std::fs;
use std::path::{Path, PathBuf};

use crible_core::config::LexicalConfig;
use crible_core::error::{CribleError, ErrorCode};
use crible_core::ids::{Generation, Iid, PassageKey};
use tantivy::directory::MmapDirectory;
use tantivy::schema::{
    Field, IndexRecordOption, NumericOptions, Schema, TextFieldIndexing, TextOptions,
};
use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, Searcher, SegmentReader, TantivyDocument, Term,
};

use crate::generation::RECORDS_FILE_NAME;
use crate::records::ReadPool;
use crate::segment::io_error;

/// Name of the tantivy directory inside an epoch directory (`docs/02`).
pub(crate) const LEXICAL_DIR_NAME: &str = "lexical";

/// Name the analysis chain is registered under in the tokenizer manager.
///
/// The name is stored in the schema of the index, so it must never change: a
/// different name would make an existing index unreadable without its analysis
/// chain, and require the rebuild job of `RECONSTRUIRE-LEXICAL`.
const TOKENIZER_NAME: &str = "crible";

/// Name of the field holding the internal identifier of a passage.
///
/// Public because the lexical branch of a query reads it as a fast field and
/// indexes nothing itself: the name is the store's, so the two cannot drift
/// apart (ADR-0089, "Rappels").
pub const IID_FIELD_NAME: &str = "iid";

/// Name of the field holding the encoder input of a passage.
///
/// Public for the same reason as [`IID_FIELD_NAME`]: the branch builds its term
/// queries on this field, and never names it a second time.
pub const TEXT_FIELD_NAME: &str = "text";

/// Number of read-only connections the lexical reader keeps on
/// `records.sqlite`.
///
/// One query reads the key of the passages it keeps after its search, in the
/// calling thread, so one connection serves it; a second would only queue on
/// the same rows. The value is a bound on connections, not a measured optimum:
/// `MES-06` is the measure that would settle it.
const KEY_READ_CONNECTIONS: usize = 1;

/// Number of indexing threads of one commit.
///
/// One writer per index serialises the commits of the index (`docs/02`,
/// "Concurrence"), so the writer needs a single thread; a second one would only
/// grow the peak memory of a commit.
const WRITER_THREADS: usize = 1;

/// Memory budget of the writer, in bytes.
///
/// Fixed by the code (`AGENTS.md` rule 12): a generation is written whole and its
/// segments are merged later, so a larger budget buys fewer, larger segments at
/// the cost of a higher peak. 50 MiB leaves room for one generation of a large
/// batch without competing with the codes and the filter structures in RAM
/// (`docs/02`, "Mémoire : ce qui est en RAM"). `MES-06` is the measure that
/// settles whether the value is right.
const WRITER_MEMORY_BUDGET_IN_BYTES: usize = 50 * 1024 * 1024;

/// State of the lexical index of one epoch, as `INV-04` reads it.
///
/// It is what a test and a recovery compare: the generation carried by the
/// payload of the last tantivy commit, and the iids the index holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LexicalState {
    /// Generation carried by the payload of the last tantivy commit.
    pub generation: Option<Generation>,
    /// iids of the indexed passages (any order).
    pub iids: Vec<Iid>,
}

/// Reads the tantivy index of an epoch, `<index>/epochs/<epoch>`.
///
/// An index the configuration does not enable, one whose epoch has no lexical
/// directory yet, and one whose writer never committed all read as
/// [`LexicalState::default()`]: no generation and no passage. A directory that
/// holds a committed index is read whole — its segments, through the `iid` fast
/// field — and a failure to read it is an [`ErrorCode::Internal`] error, since a
/// corrupt derived directory no configuration asked for is a state no commit
/// produced.
pub fn read_state(epoch_dir: &Path) -> Result<LexicalState, CribleError> {
    let directory = index_directory(epoch_dir);
    let Some(index) = existing_index(&directory)? else {
        return Ok(LexicalState::default());
    };
    let generation = committed_generation_of(&index, &directory)?;

    let mut iids = Vec::new();
    let segments = index.searchable_segments().map_err(|error| {
        tantivy_error("list the segments of the lexical index", &directory, error)
    })?;
    for segment in segments {
        let reader = SegmentReader::open(&segment)
            .map_err(|error| tantivy_error("open a lexical segment", &directory, error))?;
        let column = reader.fast_fields().u64(IID_FIELD_NAME).map_err(|error| {
            tantivy_error(
                "read the iid fast field of the lexical index",
                &directory,
                error,
            )
        })?;
        for doc in reader.doc_ids_alive() {
            let value = column.first(doc).ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "the lexical index {} holds a passage without its iid fast field",
                        directory.display()
                    ),
                )
            })?;
            iids.push(iid_of(value, &directory)?);
        }
    }
    Ok(LexicalState { generation, iids })
}

/// Reader of the lexical index of one epoch, as one published snapshot holds it.
///
/// The reader is created after the commit it reads, with
/// [`ReloadPolicy::Manual`]: no background thread watches `meta.json`, so a
/// search started on this snapshot reads the commit the snapshot names, and no
/// other (`docs/02`, "Concurrence").
///
/// It also holds the read-only connections to the `records.sqlite` of the same
/// epoch, because the lexical branch has to translate between the two sides of
/// a passage: tantivy indexes its text under an internal identifier, while the
/// answer names it by its key `doc_id#ord`, which only the reference state
/// carries (`docs/03`, "Branche lexicale"). The pool is created with the reader,
/// once per snapshot, so a query opens no connection of its own.
pub struct LexicalIndex {
    reader: IndexReader,
    records: ReadPool,
}

impl LexicalIndex {
    /// Reader over the commits of the tantivy index `index`.
    ///
    /// `directory` is the lexical directory of the epoch. The reference state
    /// sits beside it, `<epoch>/records.sqlite` (`docs/02`, "Organisation sur
    /// disque"), which [`records_path_of`] names.
    fn reader_of(index: Index, directory: &Path) -> Result<Self, CribleError> {
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|error| {
                tantivy_error("open the reader of the lexical index", directory, error)
            })?;
        reader.reload().map_err(|error| {
            tantivy_error("reload the reader of the lexical index", directory, error)
        })?;
        let records = ReadPool::new(&records_path_of(directory), KEY_READ_CONNECTIONS)?;
        Ok(Self { reader, records })
    }

    /// Searcher of the loaded generation, which the lexical branch of a query
    /// runs on (`RECHERCHER-BM25-DANS-LE-POOL`).
    #[must_use]
    pub fn searcher(&self) -> Searcher {
        self.reader.searcher()
    }

    /// Terms the analysis chain of the index makes of `query`, in order and
    /// without repetition: the bag of terms `docs/03` scores in BM25.
    ///
    /// The chain is the one `register_analyzer` put under `TOKENIZER_NAME`
    /// when the index was opened (ADR-0086), so the branch of a query never
    /// rebuilds it and never names the language: the index is the authority on
    /// how its text was analysed.
    pub fn terms(&self, query: &str) -> Result<Vec<String>, CribleError> {
        let searcher = self.reader.searcher();
        let mut analyzer = searcher
            .index()
            .tokenizers()
            .get(TOKENIZER_NAME)
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    format!(
                        "the lexical index has no analysis chain registered under \
                         '{TOKENIZER_NAME}'"
                    ),
                )
            })?;
        let mut stream = analyzer.token_stream(query);
        let mut terms: Vec<String> = Vec::new();
        while let Some(token) = stream.next() {
            if !terms.iter().any(|term| term == &token.text) {
                terms.push(token.text.clone());
            }
        }
        Ok(terms)
    }

    /// Key `doc_id#ord` of the passage `iid`, or `None` when the reference state
    /// holds no such passage.
    ///
    /// The key is the tie-break of the ranking of both branches (ADR-0045,
    /// ADR-0079): a branch reads it for the candidates it keeps, after its
    /// search, never for every candidate it scores.
    pub fn passage_key(&self, iid: Iid) -> Result<Option<PassageKey>, CribleError> {
        self.records.read_passage_key(iid)
    }

    /// Internal identifier of the passage `key`, or `None` when the reference
    /// state holds no such passage.
    ///
    /// This is the lookup `explain` needs: a passage is designated by its
    /// external key, and the derived index is reached by identifier only.
    pub fn passage_iid(&self, key: &PassageKey) -> Result<Option<Iid>, CribleError> {
        self.records.read_passage_iid(key)
    }
}

/// Path of `records.sqlite` beside the lexical directory `directory`.
///
/// The lexical directory of an epoch is `<epoch>/lexical` and the reference
/// state is `<epoch>/records.sqlite` (`docs/02`, "Organisation sur disque"), so
/// the epoch directory is its parent.
fn records_path_of(directory: &Path) -> PathBuf {
    directory.parent().map_or_else(
        || directory.to_path_buf(),
        |epoch| epoch.join(RECORDS_FILE_NAME),
    )
}

/// What the lexical index of an epoch is, relative to the generation the
/// reference state committed (`ADR-0088`, decision 5).
///
/// `observe` classifies the state once per open. The lexical branch of a query
/// reads it to choose what it does when
/// [`Snapshot::lexical`](crate::generation::Snapshot::lexical) is absent
/// (`RECHERCHER-BM25-DANS-LE-POOL`):
/// [`Behind`](LexicalStatus::Behind), [`Ahead`](LexicalStatus::Ahead),
/// [`NotCertified`](LexicalStatus::NotCertified) and
/// [`Unreadable`](LexicalStatus::Unreadable) are the four states whose refusal
/// `docs/05` names, in `overloaded` (503), with a message that quotes the state
/// found; `ADR-0089`, decision 1, adds two messages for the states without a
/// tantivy commit. [`Disabled`](LexicalStatus::Disabled) is not one of them: a
/// branch the configuration disables refuses in `invalid_request` (400)
/// (`ADR-0089`, decision 3), and [`Missing`](LexicalStatus::Missing) has no
/// passage to serve (decision 2).
///
/// The type names no `iid`, no generation of a passage and no text: it is a
/// state of the derived directory, safe to log (INV-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexicalStatus {
    /// `lexical.enabled` is false: the index has no lexical branch, and a
    /// `lexical/` an earlier configuration left behind is never read.
    Disabled,
    /// The epoch holds no committed lexical index, and no generation of it ever
    /// created or killed a passage (`M = 0`): the branch has nothing to serve.
    Missing,
    /// The lexical index describes the committed generation, `M ≤ Gt ≤ G`: a
    /// reader over the commit `Gt` renders exactly what a reader of `G` would,
    /// because the generations of `]Gt, G]` changed neither a text nor the set
    /// of live passages (`ADR-0088`, decision 2).
    Ready {
        /// Payload of the commit the reader reads, at most the committed
        /// generation.
        committed: Generation,
    },
    /// The index is behind: `Gt < G` and `M > Gt`, so a generation after the
    /// commit created a passage or tombstoned one.
    Behind {
        /// Payload of the last commit, or `None` when the epoch has no committed
        /// lexical index while passages already exist.
        committed: Option<Generation>,
    },
    /// The index is ahead: `Gt > G`, a state no commit of this epoch produced
    /// (a restored epoch without its derived directory, a directory copied by
    /// hand).
    Ahead {
        /// Payload of the last commit.
        committed: Generation,
    },
    /// The marker was never written — an epoch written before P5 was opened for
    /// writing — and the index is not at the committed generation: only
    /// `Gt = G` certifies an index this build did not write (`ADR-0088`,
    /// decision 4).
    NotCertified {
        /// Payload of the last commit, or `None` when the epoch holds no
        /// committed lexical index: without the marker, the absence of an index
        /// says nothing about the passages the epoch carries.
        committed: Option<Generation>,
    },
    /// The state cannot be read: a payload that is not a generation, a
    /// `meta.json`, a segment or a directory tantivy does not open, or a marker
    /// ahead of the committed generation (`M > G`).
    Unreadable,
}

/// State of the lexical index of an epoch, and the reader over it when the
/// state is [`LexicalStatus::Ready`].
pub(crate) struct LexicalObservation {
    /// What the index is, relative to the committed generation.
    pub(crate) status: LexicalStatus,
    /// Reader of the commit, present only when the state is
    /// [`LexicalStatus::Ready`].
    pub(crate) index: Option<LexicalIndex>,
}

impl LexicalObservation {
    /// An observation that publishes no reader.
    fn without_reader(status: LexicalStatus) -> Self {
        Self {
            status,
            index: None,
        }
    }
}

/// Classifies the lexical index of `epoch_dir` against the committed
/// `generation` and its marker `marker` (`ADR-0088`, decisions 2, 4, 5 and 6).
///
/// The reader is opened only when the state is [`LexicalStatus::Ready`]: a
/// lexical index that describes another generation than the one the reference
/// state holds would mix two generations in one snapshot (INV-02, INV-03).
/// Nothing is written, so the same call serves a read-only open (which publishes
/// the state it observed) and a write open (which then repairs it).
///
/// An unreadable state is **not** an error (`ADR-0088`, decisions 6 and 7): the
/// directory is derived, so the caller can rebuild it on a write open, and a
/// read-only open serves the reference state without the lexical branch. The
/// case and the original message are logged here, once, with the epoch.
pub(crate) fn observe(
    epoch_dir: &Path,
    config: &LexicalConfig,
    generation: Generation,
    marker: Option<Generation>,
) -> Result<LexicalObservation, CribleError> {
    if !config.enabled {
        return Ok(LexicalObservation::without_reader(LexicalStatus::Disabled));
    }
    let directory = index_directory(epoch_dir);
    let index = match existing_index(&directory) {
        Ok(Some(index)) => index,
        Ok(None) => return Ok(LexicalObservation::without_reader(no_commit_status(marker))),
        Err(error) => {
            return Ok(unreadable(
                &directory,
                "the lexical directory, its meta.json or its segments do not open",
                error,
            ));
        }
    };
    let committed = match committed_generation_of(&index, &directory) {
        Ok(committed) => committed,
        Err(error) => {
            return Ok(unreadable(
                &directory,
                "the payload of its last commit is not a generation",
                error,
            ));
        }
    };
    if let Some(marker) = marker
        && marker.get() > generation.get()
    {
        return Ok(unreadable(
            &directory,
            "its passage change marker is ahead of the committed generation",
            format!("marker {} > generation {}", marker.get(), generation.get()),
        ));
    }
    let status = status_of(committed, generation, marker);
    if !matches!(status, LexicalStatus::Ready { .. }) {
        return Ok(LexicalObservation::without_reader(status));
    }
    // A language the configuration parser accepted is a valid analysis chain;
    // an error here is a configuration one, not an unreadable index.
    register_analyzer(&index, config)?;
    match LexicalIndex::reader_of(index, &directory) {
        Ok(reader) => Ok(LexicalObservation {
            status,
            index: Some(reader),
        }),
        Err(error) => Ok(unreadable(
            &directory,
            "the segments of its last commit do not open",
            error,
        )),
    }
}

/// The status of an epoch that has no committed lexical index.
///
/// An absent marker and a marker of `0` are not the same state: `0` says the
/// epoch never carried a passage, so an empty branch is complete and
/// [`LexicalStatus::Missing`] is honest, while an absent column says the epoch
/// was written before P5 and certifies nothing — the passages may be there and
/// the index may simply never have existed (`ADR-0088`, decision 4).
fn no_commit_status(marker: Option<Generation>) -> LexicalStatus {
    match marker.map(|marker| marker.get()) {
        None => LexicalStatus::NotCertified { committed: None },
        // No passage ever created or killed: nothing to serve.
        Some(0) => LexicalStatus::Missing,
        // Passages exist and no lexical index describes them.
        Some(_) => LexicalStatus::Behind { committed: None },
    }
}

/// Classifies a readable index of payload `committed` against `generation`.
///
/// The caller has already refused a marker ahead of the committed generation.
fn status_of(
    committed: Option<Generation>,
    generation: Generation,
    marker: Option<Generation>,
) -> LexicalStatus {
    let Some(committed) = committed else {
        return no_commit_status(marker);
    };
    if committed.get() > generation.get() {
        return LexicalStatus::Ahead { committed };
    }
    match marker {
        // The narrow rule of ADR-0088, decision 4: without the marker, only an
        // index at the committed generation is certified.
        None if committed == generation => LexicalStatus::Ready { committed },
        None => LexicalStatus::NotCertified {
            committed: Some(committed),
        },
        Some(marker) if marker.get() <= committed.get() => LexicalStatus::Ready { committed },
        Some(_) => LexicalStatus::Behind {
            committed: Some(committed),
        },
    }
}

/// An observation of an unreadable lexical state, logged with its origin.
fn unreadable(directory: &Path, case: &str, error: impl std::fmt::Display) -> LexicalObservation {
    tracing::warn!(
        lexical_index = %directory.display(),
        case,
        error = %error,
        "the lexical index is unreadable: a write open rebuilds it, a read-only open serves \
         the reference state without the lexical branch (ADR-0088)"
    );
    LexicalObservation::without_reader(LexicalStatus::Unreadable)
}

/// Writer of the lexical index of one epoch, for the commit of one generation.
pub(crate) struct LexicalWriter {
    /// The tantivy index the writer commits to, kept for the reader of the
    /// commit this writer ends.
    index: Index,
    /// Directory of the index, for the error messages.
    directory: PathBuf,
    writer: IndexWriter,
    iid_field: Field,
    text_field: Field,
}

impl LexicalWriter {
    /// Indexes one passage under its internal identifier.
    ///
    /// The text is the encoder input of the passage (`docs/03`), never its
    /// identifier: a query on `iid` is what the collector of the lexical branch
    /// uses, not a query on a document text.
    pub(crate) fn add(&self, iid: Iid, text: &str) -> Result<(), CribleError> {
        let mut document = TantivyDocument::new();
        document.add_u64(self.iid_field, u64::from(iid.get()));
        document.add_text(self.text_field, text);
        self.writer
            .add_document(document)
            .map(|_opstamp| ())
            .map_err(|error| {
                tantivy_error("add a passage to the lexical index", &self.directory, error)
            })
    }

    /// Removes the passage indexed under `iid`, if it is present.
    ///
    /// The deletion takes effect at the next commit, and deleting an absent iid
    /// is a no-op: the catch-up of a restart applies the tombstones of the
    /// generations it replays without reading the index first.
    pub(crate) fn delete(&self, iid: Iid) {
        self.writer
            .delete_term(Term::from_field_u64(self.iid_field, u64::from(iid.get())));
    }

    /// Commits the changes of this writer with `generation` as their payload.
    ///
    /// The commit is synchronous: when it returns, tantivy has published the new
    /// state on disk, with the payload the restart reads.
    pub(crate) fn commit(&mut self, generation: Generation) -> Result<(), CribleError> {
        let mut prepared = self.writer.prepare_commit().map_err(|error| {
            tantivy_error(
                "prepare the commit of the lexical index",
                &self.directory,
                error,
            )
        })?;
        prepared.set_payload(&payload(generation));
        prepared
            .commit()
            .map_err(|error| tantivy_error("commit the lexical index", &self.directory, error))?;
        Ok(())
    }

    /// Reader of the state this writer has committed.
    pub(crate) fn reader(&self) -> Result<LexicalIndex, CribleError> {
        LexicalIndex::reader_of(self.index.clone(), &self.directory)
    }
}

/// Writer of the lexical index of `epoch_dir`, creating the index when the epoch
/// has none.
///
/// The epoch directory must exist: the writer only ever creates `lexical/`
/// inside it, never the epoch itself.
pub(crate) fn writer(
    epoch_dir: &Path,
    config: &LexicalConfig,
) -> Result<LexicalWriter, CribleError> {
    let directory = index_directory(epoch_dir);
    let index = match existing_index(&directory)? {
        Some(index) => index,
        None => {
            fs::create_dir_all(&directory)
                .map_err(|error| io_error("create the lexical directory", &directory, &error))?;
            Index::create_in_dir(&directory, schema(config.positions))
                .map_err(|error| tantivy_error("create the lexical index", &directory, error))?
        }
    };
    register_analyzer(&index, config)?;
    let iid_field = field_of(&index, IID_FIELD_NAME, &directory)?;
    let text_field = field_of(&index, TEXT_FIELD_NAME, &directory)?;
    let writer = index
        .writer_with_num_threads(WRITER_THREADS, WRITER_MEMORY_BUDGET_IN_BYTES)
        .map_err(|error| {
            tantivy_error("open the writer of the lexical index", &directory, error)
        })?;
    Ok(LexicalWriter {
        index,
        directory,
        writer,
        iid_field,
        text_field,
    })
}

/// Removes the whole lexical directory of an epoch, for a rebuild.
///
/// The directory is derived: removing it loses nothing the reference state does
/// not carry, and an absent directory is not an error.
pub(crate) fn remove_index(epoch_dir: &Path) -> Result<(), CribleError> {
    let directory = index_directory(epoch_dir);
    match fs::remove_dir_all(&directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove the lexical index", &directory, &error)),
    }
}

/// Directory of the lexical index inside the epoch directory `epoch_dir`.
pub(crate) fn index_directory(epoch_dir: &Path) -> PathBuf {
    epoch_dir.join(LEXICAL_DIR_NAME)
}

/// Schema of a lexical index, with term positions only when `positions` asks for
/// them.
fn schema(positions: bool) -> Schema {
    let record = if positions {
        IndexRecordOption::WithFreqsAndPositions
    } else {
        IndexRecordOption::WithFreqs
    };
    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer(TOKENIZER_NAME)
        .set_index_option(record);
    let mut builder = Schema::builder();
    builder.add_u64_field(
        IID_FIELD_NAME,
        NumericOptions::default().set_indexed().set_fast(),
    );
    builder.add_text_field(
        TEXT_FIELD_NAME,
        TextOptions::default().set_indexing_options(text_indexing),
    );
    builder.build()
}

/// Opens the tantivy index of `directory`, or reports that it holds none.
///
/// A committed index is one whose `meta.json` exists, which is what tantivy
/// itself checks ([`Index::exists`]). An absent directory, and a directory a
/// writer created before its first commit, both report `None`.
fn existing_index(directory: &Path) -> Result<Option<Index>, CribleError> {
    if !directory.is_dir() {
        return Ok(None);
    }
    let mmap = MmapDirectory::open(directory)
        .map_err(|error| tantivy_error("open the lexical directory", directory, error))?;
    let exists = Index::exists(&mmap)
        .map_err(|error| tantivy_error("look for the lexical index", directory, error))?;
    if !exists {
        return Ok(None);
    }
    let index = Index::open(mmap)
        .map_err(|error| tantivy_error("open the lexical index", directory, error))?;
    Ok(Some(index))
}

/// Registers the analysis chain of `config` in the tokenizer manager of `index`.
///
/// The tokenizer manager already holds the default chain of tantivy; the chain
/// of `lexical.language` takes the name [`TOKENIZER_NAME`], the one the schema of
/// the index refers to.
fn register_analyzer(index: &Index, config: &LexicalConfig) -> Result<(), CribleError> {
    let chain = analyzer::analyzer(&config.language)?;
    index.tokenizers().register(TOKENIZER_NAME, chain);
    Ok(())
}

/// Field `name` of the schema of `index`, or an internal error.
///
/// The schema is read from the index, not rebuilt from the configuration, so an
/// identifier is always the one the stored documents use.
fn field_of(index: &Index, name: &str, directory: &Path) -> Result<Field, CribleError> {
    index.schema().get_field(name).map_err(|error| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "the lexical index {} has no field '{name}': {error}",
                directory.display()
            ),
        )
    })
}

/// The generation carried by the payload of the last commit of `index`.
///
/// `None` when the index holds no committed payload: a directory a writer
/// created before its first commit, which describes no generation. A payload
/// this build cannot read is an error, not `None`: it means another build wrote
/// the index, and the state is then undecidable. [`observe`] turns that error
/// into [`LexicalStatus::Unreadable`], which a write open rebuilds (`ADR-0088`,
/// decision 6) and a read-only open only reports (decision 7).
fn committed_generation_of(
    index: &Index,
    directory: &Path,
) -> Result<Option<Generation>, CribleError> {
    let meta = index.load_metas().map_err(|error| {
        tantivy_error("read the metadata of the lexical index", directory, error)
    })?;
    meta.payload
        .as_deref()
        .map(|payload| parse_payload(payload, directory))
        .transpose()
}

/// Payload of a commit: the decimal number of `generation`.
fn payload(generation: Generation) -> String {
    generation.get().to_string()
}

/// Generation carried by the payload `payload`, or an internal error.
///
/// A payload this build cannot read means the lexical index was written by
/// another one, or is corrupt: `Gt` is undecidable, so nothing can be caught up
/// and the derived directory has to be rebuilt. [`observe`] reports the state as
/// [`LexicalStatus::Unreadable`] and logs the message built here.
fn parse_payload(payload: &str, directory: &Path) -> Result<Generation, CribleError> {
    let trimmed = payload.trim();
    trimmed
        .parse::<u64>()
        .map(Generation::new)
        .map_err(|error| {
            CribleError::new(
                ErrorCode::Internal,
                format!(
                    "the payload {:?} of the lexical index {} is not a generation: {error}",
                    payload,
                    directory.display()
                ),
            )
        })
}

/// The `u32` internal identifier held by the `iid` fast field value `value`.
fn iid_of(value: u64, directory: &Path) -> Result<Iid, CribleError> {
    u32::try_from(value).map(Iid::new).map_err(|_| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "the lexical index {} holds the iid fast field value {value}, which is not a \
                 Crible internal identifier",
                directory.display()
            ),
        )
    })
}

/// Builds an internal error naming the lexical index and the failed operation.
///
/// The failing type is the one tantivy returned — [`tantivy::TantivyError`],
/// an `OpenDirectoryError`, an `OpenReadError` — so it is taken as any display,
/// not converted: the message must name what tantivy refused.
fn tantivy_error(action: &str, directory: &Path, error: impl std::fmt::Display) -> CribleError {
    CribleError::new(
        ErrorCode::Internal,
        format!("cannot {action} {}: {error}", directory.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        LexicalState, index_directory, parse_payload, payload, read_state, remove_index, writer,
    };
    use crible_core::config::LexicalConfig;
    use crible_core::error::ErrorCode;
    use crible_core::ids::{Generation, Iid};

    /// Configuration of the fixture lexical index: French, no positions.
    fn config() -> LexicalConfig {
        LexicalConfig {
            enabled: true,
            language: "fr".to_owned(),
            positions: false,
        }
    }

    /// The payload of a commit is the decimal number of its generation, in both
    /// directions: the restart compares what the commit wrote.
    #[test]
    fn the_payload_of_a_commit_is_the_generation_it_indexed() {
        let directory = std::path::Path::new("lexical");
        let text = payload(Generation::new(412));
        assert_eq!(text, "412");
        let parsed = parse_payload(&text, directory)
            .unwrap_or_else(|error| panic!("'412' must parse as a generation: {error}"));
        assert_eq!(parsed, Generation::new(412));
        let error = match parse_payload("gen-412", directory) {
            Ok(generation) => panic!("'gen-412' is not a generation, got {generation:?}"),
            Err(error) => error,
        };
        assert_eq!(error.code(), ErrorCode::Internal);
        assert!(error.message().contains("gen-412"), "{}", error.message());
    }

    /// An epoch without a lexical directory reads as the default state, never as
    /// an error: a configuration that disables the branch creates none.
    #[test]
    fn an_absent_lexical_index_reads_as_the_default_state() {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let state = read_state(temp.path()).unwrap_or_else(|error| {
            panic!("an absent lexical index must not be an error: {error}")
        });
        assert_eq!(
            state,
            LexicalState::default(),
            "an epoch without lexical/ must read as no generation and no passage"
        );
        assert!(
            !index_directory(temp.path()).exists(),
            "reading must create nothing"
        );
    }

    /// One commit carries both its generation and the passages it indexed, and a
    /// removed index reads as the default state again.
    #[test]
    fn a_commit_publishes_its_generation_and_its_iids() {
        let temp = tempfile::tempdir()
            .unwrap_or_else(|error| panic!("a temporary directory must be creatable: {error}"));
        let epoch_dir = temp.path().join("epochs").join("1");
        std::fs::create_dir_all(&epoch_dir)
            .unwrap_or_else(|error| panic!("the epoch directory must be creatable: {error}"));

        let mut lexical_writer = writer(&epoch_dir, &config())
            .unwrap_or_else(|error| panic!("the lexical writer must open: {error}"));
        lexical_writer
            .add(Iid::new(0), "l'article 1240 du code civil")
            .unwrap_or_else(|error| panic!("the passage must be indexed: {error}"));
        lexical_writer
            .add(Iid::new(1), "L. 1132-1")
            .unwrap_or_else(|error| panic!("the passage must be indexed: {error}"));
        lexical_writer
            .commit(Generation::new(3))
            .unwrap_or_else(|error| panic!("the lexical commit must succeed: {error}"));
        drop(lexical_writer);

        let state = read_state(&epoch_dir)
            .unwrap_or_else(|error| panic!("the lexical index must be readable: {error}"));
        assert_eq!(state.generation, Some(Generation::new(3)));
        let mut iids: Vec<u32> = state.iids.iter().map(|iid| iid.get()).collect();
        iids.sort_unstable();
        assert_eq!(iids, vec![0, 1]);

        remove_index(&epoch_dir)
            .unwrap_or_else(|error| panic!("the lexical index must be removable: {error}"));
        let state = read_state(&epoch_dir)
            .unwrap_or_else(|error| panic!("a removed lexical index must read as absent: {error}"));
        assert_eq!(state, LexicalState::default());
    }
}
