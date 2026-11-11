// SPDX-License-Identifier: Apache-2.0

//! Prints the bag of terms a lexical query is scored with, and the document
//! frequency of each of them in a lexical index.
//!
//! The measure of ADR-0148 (MES-11, campaign T14) has to explain a rank, and a
//! rank alone does not say whether a document is missing from the answer
//! because the query never produced the canonical term, or because the term
//! exists but the other pieces of the query dilute the score. `crible search`
//! answers with documents only; `explain_ids` of the route is not exposed by the
//! CLI. This example reads the two facts the engine already holds: the terms of
//! [`analyzer`] for the configuration asked on the command line, and the
//! `doc_freq` of each of them in a `lexical/` directory. It changes nothing.
//!
//! ```sh
//! cargo run --release -p crible-store --example lexical_terms -- \
//!     data/t14-mes11/data/indexes/articles/epochs/1/lexical v1 "L. 1132-1"
//! ```
//!
//! Output: one JSON object on one line, with `query`, `identifier_forms`,
//! `terms` (from the analysis chain) and `doc_freq`. **Read a copy or a closed
//! index**: opening the directory takes the blocking `META_LOCK` of Tantivy,
//! exactly as `lexical_tokens.rs` explains.

use crible_core::config::LexicalIdentifiers;
use crible_store::lexical::analyzer::analyzer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .ok_or("usage: <lexical-dir> <none|v1> <query>")?;
    let forms = arguments
        .next()
        .ok_or("usage: <lexical-dir> <none|v1> <query>")?;
    let query = arguments
        .next()
        .ok_or("usage: <lexical-dir> <none|v1> <query>")?;

    let identifiers = match forms.as_str() {
        "none" => LexicalIdentifiers::None,
        "v1" => LexicalIdentifiers::V1,
        other => return Err(format!("unknown identifier domain '{other}'").into()),
    };

    let mut chain = analyzer("fr", identifiers)?;
    let mut stream = chain.token_stream(&query);
    let mut terms: Vec<String> = Vec::new();
    while let Some(token) = stream.next() {
        if !terms.iter().any(|term| term == &token.text) {
            terms.push(token.text.clone());
        }
    }

    let index = tantivy::Index::open_in_dir(std::path::Path::new(&directory))?;
    let field = index.schema().get_field("text")?;
    let searcher = index.reader()?.searcher();
    let mut frequencies = Vec::new();
    for term in &terms {
        let frequency = searcher.doc_freq(&tantivy::Term::from_field_text(field, term))?;
        frequencies.push(serde_json::json!({"term": term, "doc_freq": frequency}));
    }

    println!(
        "{}",
        serde_json::json!({
            "query": query,
            "identifier_forms": forms,
            "terms": terms,
            "doc_freq": frequencies,
            "docs": searcher.num_docs(),
        })
    );
    Ok(())
}
