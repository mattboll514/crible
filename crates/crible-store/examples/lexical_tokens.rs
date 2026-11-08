// SPDX-License-Identifier: Apache-2.0

//! Counts the tokens analysed by a lexical index of Crible.
//!
//! `ADR-0139`, decision 4, asks for "le nombre de passages et de jetons
//! analysés" during a lexical rebuild. `crible rebuild-lexical` prints neither,
//! and no metric carries them; but the lexical index is a Tantivy index, and
//! Tantivy records the total number of tokens per field and per segment for its
//! BM25 statistics. This example reads that total, without touching the engine.
//!
//! ```sh
//! cargo run --release -p crible-store --example lexical_tokens -- \
//!     data/p10-import/lexical/dole data/p10-import/lexical/sarde
//! ```
//!
//! Each argument is a `lexical/` directory. The output is one JSON object per
//! directory, on one line: `path`, `field`, `segments`, `docs`, `tokens`.
//!
//! **Read a copy, never a served index.** Reading the totals opens a Tantivy
//! reader (`index.reader()`), which takes `META_LOCK`, the blocking lock of
//! `.tantivy-meta.lock`, so that the segment files it opens are not garbage
//! collected under it. On a copy (the campaign copied the six `lexical/`
//! directories under `data/p10-import/lexical/`) that is harmless. On an index
//! a server serves, or on a volume opened for writing, it takes a lock of a live
//! index, and a segment the writer deletes has no stable total; copy the
//! directory first. The lock of the writer is another file,
//! `.tantivy-writer.lock` (`INDEX_WRITER_LOCK`), and it belongs to the
//! `IndexWriter`, which this example never opens.

use std::path::Path;

use tantivy::query::Bm25StatisticsProvider;

fn count(path: &Path) -> Result<(usize, u64, u64), Box<dyn std::error::Error>> {
    let index = tantivy::Index::open_in_dir(path)?;
    let field = index
        .schema()
        .get_field("text")
        .map_err(|_| format!("{}: no `text` field in the schema", path.display()))?;
    let searcher = index.reader()?.searcher();
    let tokens = searcher.total_num_tokens(field)?;
    let docs = searcher.total_num_docs()?;
    Ok((searcher.segment_readers().len(), docs, tokens))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() {
        return Err("usage: lexical_tokens <lexical-dir> [<lexical-dir>...]".into());
    }
    for argument in arguments {
        let path = Path::new(&argument);
        let (segments, docs, tokens) = count(path)?;
        println!(
            "{{\"path\":\"{}\",\"field\":\"text\",\"segments\":{},\"docs\":{},\"tokens\":{}}}",
            path.display(),
            segments,
            docs,
            tokens
        );
    }
    Ok(())
}
