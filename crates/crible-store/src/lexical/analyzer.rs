// SPDX-License-Identifier: Apache-2.0

//! Analysis chain of the lexical branch, built from `lexical.language`.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale (tantivy)"), ADR-0007
//! (tantivy for BM25) and ADR-0086 (domain of `lexical.language` and the chain
//! of the languages without a stop-word list).
//!
//! # Chain
//!
//! The text indexed by tantivy is the encoder input, that is exactly the text
//! of the dense branch. It goes through, in this order:
//!
//! 1. `SimpleTokenizer`, which splits on every character that is not
//!    alphanumeric;
//! 2. `RemoveLongFilter(40)`, which drops the tokens of 40 UTF-8 bytes or more
//!    (the limit counts bytes, not characters);
//! 3. `LowerCaser`, then `AsciiFoldingFilter`, so that `é` and `e` meet;
//! 4. `StopWordFilter`, absent for the five languages tantivy has no list for
//!    (`ar`, `el`, `ro`, `ta`, `tr`);
//! 5. `Stemmer`, which reduces each token to its stem.
//!
//! # Languages
//!
//! A language is designated by a lowercase ISO 639-1 code from
//! [`crible_core::config::LEXICAL_LANGUAGES`] (ADR-0086). The configuration
//! validation of `crible-core` refuses every other value, so an unknown code
//! reaching this module is a bug, reported as an internal error.

use crible_core::config::LEXICAL_LANGUAGES;
use crible_core::error::{CribleError, ErrorCode};
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer,
    StopWordFilter, TextAnalyzer,
};

/// Longest token kept by the chain, in UTF-8 bytes (`RemoveLongFilter(40)`): a
/// token of `MAX_TOKEN_LENGTH` bytes or more is dropped.
const MAX_TOKEN_LENGTH: usize = 40;

/// Builds the analysis chain of the index whose `lexical.language` is `code`.
///
/// The five languages without a stop-word list get an empty one, which removes
/// nothing: their tokens are exactly those of a chain without that filter
/// (ADR-0086).
pub fn analyzer(code: &str) -> Result<TextAnalyzer, CribleError> {
    let language = tantivy_language(code).ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            format!(
                "analysis language '{code}' is not one of the {} codes of ADR-0086; \
                 the configuration validation should have refused it",
                LEXICAL_LANGUAGES.len()
            ),
        )
    })?;
    let stop_words =
        StopWordFilter::new(language).unwrap_or_else(|| StopWordFilter::remove(Vec::new()));

    Ok(TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(MAX_TOKEN_LENGTH))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(stop_words)
        .filter(Stemmer::new(language))
        .build())
}

/// tantivy language of an ISO 639-1 code of the table of ADR-0086.
///
/// tantivy exposes no code, only named variants, so this mapping is ours. It is
/// exhaustive over [`LEXICAL_LANGUAGES`]; a test keeps the two aligned.
fn tantivy_language(code: &str) -> Option<Language> {
    let language = match code {
        "ar" => Language::Arabic,
        "da" => Language::Danish,
        "de" => Language::German,
        "el" => Language::Greek,
        "en" => Language::English,
        "es" => Language::Spanish,
        "fi" => Language::Finnish,
        "fr" => Language::French,
        "hu" => Language::Hungarian,
        "it" => Language::Italian,
        "nl" => Language::Dutch,
        "no" => Language::Norwegian,
        "pt" => Language::Portuguese,
        "ro" => Language::Romanian,
        "ru" => Language::Russian,
        "sv" => Language::Swedish,
        "ta" => Language::Tamil,
        "tr" => Language::Turkish,
        _ => return None,
    };
    Some(language)
}

#[cfg(test)]
mod tests {
    use super::{analyzer, tantivy_language};
    use crible_core::config::LEXICAL_LANGUAGES;
    use crible_core::error::ErrorCode;
    use tantivy::tokenizer::Language;

    /// Tokens the chain of `code` produces for `text`, in order.
    fn tokens(code: &str, text: &str) -> Vec<String> {
        let mut analyzer = analyzer(code).expect("the code is one of the table");
        let mut stream = analyzer.token_stream(text);
        let mut tokens = Vec::new();
        while let Some(token) = stream.next() {
            tokens.push(token.text.clone());
        }
        tokens
    }

    /// `docs/03` gives `"fr"` as the only example: the French chain is the one
    /// the whole phase is measured on, hence the legal phrases below.
    ///
    /// The tokens are the report of the task; the problematic cases they show
    /// are collected for MES-11 and not corrected here.
    #[test]
    fn french_legal_phrase_tokens() {
        // `l'` is split by the tokenizer, and `l` then disappears: it is a
        // French stop word, not because elisions are handled (MES-11).
        assert_eq!(
            tokens("fr", "l'article 1240 du code civil"),
            ["articl", "1240", "cod", "civil"]
        );
    }

    /// An identifier loses its letter prefix and its dash: only the numbers
    /// survive (MES-11, Q-04).
    #[test]
    fn french_identifier_tokens() {
        assert_eq!(tokens("fr", "L. 1132-1"), ["1132", "1"]);
    }

    /// Both halves of the elision are French stop words, so the whole phrase
    /// analyses to nothing (MES-11, Q-04).
    #[test]
    fn french_elision_tokens() {
        assert!(tokens("fr", "qu'il").is_empty());
    }

    /// The compound is split and never joined back (MES-11, Q-04).
    #[test]
    fn french_compound_tokens() {
        assert_eq!(tokens("fr", "dommages-intérêts"), ["dommag", "interet"]);
    }

    /// ADR-0086: `ar`, `el`, `ro`, `ta` and `tr` have no stop-word list, so
    /// their tokens are those of a chain without that filter: French stop words
    /// survive in every one of them.
    #[test]
    fn a_language_without_a_stop_word_list_keeps_every_token() {
        for code in ["ar", "el", "ro", "ta", "tr"] {
            assert_eq!(
                tokens(code, "de la"),
                ["de", "la"],
                "{code} has no stop-word list, so the French stop words must survive"
            );
        }
        assert_eq!(tokens("tr", "qu'il"), ["qu", "il"]);
    }

    /// Every code of the table of ADR-0086 has a tantivy language, no other code
    /// has one, and the eighteen code-language pairs are two to two distinct: a
    /// duplicated entry of `LEXICAL_LANGUAGES`, or an entry that is not one of
    /// the six codes sampled below, cannot pass in silence.
    #[test]
    fn every_code_of_the_table_maps_to_a_tantivy_language() {
        use std::collections::HashSet;

        assert_eq!(LEXICAL_LANGUAGES.len(), 18);
        let mut languages = HashSet::with_capacity(LEXICAL_LANGUAGES.len());
        for code in LEXICAL_LANGUAGES {
            let language = match tantivy_language(code) {
                Some(language) => language,
                None => panic!("{code} is in the table but has no tantivy language"),
            };
            assert!(
                languages.insert(language),
                "{code} maps to a tantivy language another code already maps to"
            );
        }
        assert_eq!(
            languages.len(),
            LEXICAL_LANGUAGES.len(),
            "the {} codes must map to {} distinct languages",
            LEXICAL_LANGUAGES.len(),
            LEXICAL_LANGUAGES.len()
        );
        for code in ["", "FR", "French", "nb", "gr", "xx"] {
            assert_eq!(
                tantivy_language(code),
                None,
                "{code} is not in the table and must not map"
            );
        }
    }

    /// ADR-0086 fixes two codes that the ISO 639-1 family and the tantivy names
    /// would not suggest on their own.
    #[test]
    fn no_is_norwegian_and_el_is_greek() {
        assert_eq!(tantivy_language("no"), Some(Language::Norwegian));
        assert_eq!(tantivy_language("el"), Some(Language::Greek));
    }

    /// A code the configuration validation should have refused is an internal
    /// error here, never a panic and never a silent default (ADR-0086).
    #[test]
    fn an_unknown_code_is_an_internal_error() {
        let error = match analyzer("FR") {
            Ok(_) => panic!("FR is not a code of the table"),
            Err(error) => error,
        };
        assert_eq!(error.code(), ErrorCode::Internal);
        assert!(error.message().contains("FR"), "{}", error.message());
    }
}
