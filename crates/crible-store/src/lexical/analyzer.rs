// SPDX-License-Identifier: Apache-2.0

//! Analysis chain of the lexical branch, built from `lexical.language` and
//! `lexical.identifiers`.
//!
//! References: `docs/03-recherche.md` ("Branche lexicale (tantivy)"), ADR-0007
//! (tantivy for BM25), ADR-0086 (domain of `lexical.language` and the chain of
//! the languages without a stop-word list) and ADR-0148 (canonical token of an
//! identifier, behind `lexical.identifiers`).
//!
//! # Chain
//!
//! The text indexed by tantivy is the encoder input, that is exactly the text
//! of the dense branch. It goes through, in this order:
//!
//! 1. `IdentifierTokenizer`, which wraps [`SimpleTokenizer`] — it splits on
//!    every character that is not alphanumeric — and, when
//!    `lexical.identifiers` is `"v1"`, appends the canonical token of each
//!    identifier it recognises (ADR-0148, decisions 1 to 4);
//! 2. `RemoveLongFilter(40)`, which drops the tokens of 40 UTF-8 bytes or more
//!    (the limit counts bytes, not characters);
//! 3. `LowerCaser`, then `AsciiFoldingFilter`, so that `é` and `e` meet;
//! 4. `StopWordFilter`, absent for the five languages tantivy has no list for
//!    (`ar`, `el`, `ro`, `ta`, `tr`);
//! 5. `Stemmer`, which reduces each token to its stem.
//!
//! With `lexical.identifiers = "none"`, the tokenizer is exactly
//! [`SimpleTokenizer`]: it recognises nothing and adds nothing, so the tokens
//! are those of the 1.0 chain (ADR-0148, decision 5).
//!
//! # Languages
//!
//! A language is designated by a lowercase ISO 639-1 code from
//! [`crible_core::config::LEXICAL_LANGUAGES`] (ADR-0086). The configuration
//! validation of `crible-core` refuses every other value, so an unknown code
//! reaching this module is a bug, reported as an internal error.

use crible_core::config::{LEXICAL_LANGUAGES, LexicalIdentifiers};
use crible_core::error::{CribleError, ErrorCode};
use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter, SimpleTokenStream, SimpleTokenizer,
    Stemmer, StopWordFilter, TextAnalyzer, Token, TokenStream, Tokenizer,
};

/// Longest token kept by the chain, in UTF-8 bytes (`RemoveLongFilter(40)`): a
/// token of `MAX_TOKEN_LENGTH` bytes or more is dropped.
const MAX_TOKEN_LENGTH: usize = 40;

/// Builds the analysis chain of the index whose `lexical.language` is `code`
/// and whose `lexical.identifiers` is `identifiers`.
///
/// The five languages without a stop-word list get an empty one, which removes
/// nothing: their tokens are exactly those of a chain without that filter
/// (ADR-0086).
pub fn analyzer(code: &str, identifiers: LexicalIdentifiers) -> Result<TextAnalyzer, CribleError> {
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

    Ok(TextAnalyzer::builder(IdentifierTokenizer::new(identifiers))
        .filter(RemoveLongFilter::limit(MAX_TOKEN_LENGTH))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(stop_words)
        .filter(Stemmer::new(language))
        .build())
}

/// Tokenizer of the chain: [`SimpleTokenizer`], plus the canonical token of
/// each identifier it recognises (ADR-0148, decisions 2 and 3).
///
/// It is a tokenizer and not a token filter because a filter receives the text
/// and the offsets of one token only, and neither tells a `-` from a `.` nor
/// says whether a separator was there at all (ADR-0148, decision 2).
#[derive(Clone, Default)]
struct IdentifierTokenizer {
    /// Version of the identifier forms; [`LexicalIdentifiers::None`] is the
    /// 1.0 tokenizer, which recognises nothing.
    identifiers: LexicalIdentifiers,
    inner: SimpleTokenizer,
}

impl IdentifierTokenizer {
    /// The chain of `identifiers`.
    fn new(identifiers: LexicalIdentifiers) -> Self {
        Self {
            identifiers,
            inner: SimpleTokenizer::default(),
        }
    }
}

impl Tokenizer for IdentifierTokenizer {
    type TokenStream<'a> = IdentifierTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        let canonicals = match self.identifiers {
            LexicalIdentifiers::None => Vec::new(),
            LexicalIdentifiers::V1 => canonical_tokens(text),
        };
        let mut canonicals = canonicals.into_iter();
        let next = canonicals.next();
        IdentifierTokenStream {
            inner: self.inner.token_stream(text),
            canonicals,
            next,
            queued: None,
            current: Token::default(),
        }
    }
}

/// Stream of [`IdentifierTokenizer`]: the pieces of [`SimpleTokenizer`], each
/// identifier followed by its canonical token.
///
/// The canonical token is emitted **after** the last piece of its identifier,
/// at the position of the first one, and it does not move the positions of the
/// tokens that follow (ADR-0148, decision 4). The pieces are therefore exactly
/// those of the 1.0 chain, in the same order, with the same positions.
struct IdentifierTokenStream<'a> {
    inner: SimpleTokenStream<'a>,
    /// Canonical tokens not reached yet, in the order of their identifiers.
    canonicals: std::vec::IntoIter<CanonicalToken>,
    /// First canonical token of [`Self::canonicals`], looked at before it is
    /// emitted.
    next: Option<CanonicalToken>,
    /// Canonical token to emit before pulling the next piece.
    queued: Option<Token>,
    current: Token,
}

impl TokenStream for IdentifierTokenStream<'_> {
    fn advance(&mut self) -> bool {
        if let Some(token) = self.queued.take() {
            self.current = token;
            return true;
        }
        if !self.inner.advance() {
            return false;
        }
        // The piece is copied into `current` rather than borrowed from the inner
        // stream: a canonical token, which belongs to this stream, is the next
        // token as often as a piece. The text buffer is reused, so a passage does
        // not allocate one `String` per token.
        let piece = self.inner.token();
        self.current.offset_from = piece.offset_from;
        self.current.offset_to = piece.offset_to;
        self.current.position = piece.position;
        self.current.position_length = piece.position_length;
        self.current.text.clear();
        self.current.text.push_str(&piece.text);
        let Some(next) = self.next.as_mut() else {
            return true;
        };
        // The first piece of the identifier carries the position of the whole
        // span: `from` is the first byte of the identifier, and a piece starts
        // there by construction of the two forms.
        if next.position.is_none() && next.from == self.current.offset_from {
            next.position = Some(self.current.position);
        }
        if self.current.offset_to < next.to {
            return true;
        }
        let Some(next) = self.next.take() else {
            return true;
        };
        // A piece holds every byte of the identifier only inside the span, so
        // the position is recorded before this point; the current position is
        // the fallback of a stream that did not see the first piece.
        let position = next.position.unwrap_or(self.current.position);
        self.queued = Some(Token {
            offset_from: next.from,
            offset_to: next.to,
            position,
            text: next.text,
            position_length: 1,
        });
        self.next = self.canonicals.next();
        true
    }

    fn token(&self) -> &Token {
        &self.current
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.current
    }
}

/// A canonical token an identifier of `text` adds to the 1.0 stream.
#[derive(Debug, Clone)]
struct CanonicalToken {
    /// Byte offset of the first character of the identifier.
    from: usize,
    /// Byte offset one past its last character.
    to: usize,
    /// Canonical text, with the case and the accents of the original: the
    /// `LowerCaser` and the `AsciiFoldingFilter` normalise it afterwards, as
    /// they normalise the pieces (ADR-0148, decision 4).
    text: String,
    /// Position of the first piece, recorded when the stream reaches it.
    position: Option<usize>,
}

/// Canonical tokens of the identifiers of the forms `v1` found in `text`, in
/// the order of their first character (ADR-0148, decision 3).
///
/// A match is consumed whole, so an identifier nested in another one — the
/// `1132-1` inside `L. 1132-1` — adds no second canonical token.
fn canonical_tokens(text: &str) -> Vec<CanonicalToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        // Every ASCII byte is a character boundary, so the byte scan can only
        // start a match on a boundary, and the boundary checks below slice
        // valid `&str`.
        let matched = match bytes[offset] {
            byte if byte.is_ascii_alphabetic() => article_at(text, bytes, offset),
            byte if byte.is_ascii_digit() => number_at(text, bytes, offset),
            _ => None,
        };
        match matched {
            Some((end, canonical)) => {
                tokens.push(CanonicalToken {
                    from: offset,
                    to: end,
                    text: canonical,
                    position: None,
                });
                offset = end;
            }
            None => offset += 1,
        }
    }
    tokens
}

/// The article form of ADR-0148, decision 3, if it starts at `start`.
///
/// Form A is a letter, at most one `.` and at most one whitespace character,
/// then digits, then `(-\d+)*`; the canonical token drops the dot and the
/// whitespace. An identifier the 1.0 tokenizer already keeps whole adds
/// nothing, since the canonical token would repeat the piece: `C100123` is of
/// this form and yields no token.
fn article_at(text: &str, bytes: &[u8], start: usize) -> Option<(usize, String)> {
    if !starts_a_word(text, start) {
        return None;
    }
    let mut cursor = start + 1;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
    }
    if let Some(next) = text[cursor..].chars().next()
        && next.is_whitespace()
    {
        cursor += next.len_utf8();
    }
    let digits_start = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor == digits_start {
        return None;
    }
    while bytes.get(cursor) == Some(&b'-') {
        let mut group_end = cursor + 1;
        while bytes.get(group_end).is_some_and(u8::is_ascii_digit) {
            group_end += 1;
        }
        if group_end == cursor + 1 {
            break;
        }
        cursor = group_end;
    }
    if !ends_a_word(text, cursor) {
        return None;
    }
    let mut canonical = String::with_capacity(cursor - start);
    let mut has_separator = false;
    for character in text[start..cursor].chars() {
        has_separator |= character == '.' || character == '-' || character.is_whitespace();
        if character != '.' && !character.is_whitespace() {
            canonical.push(character);
        }
    }
    if !has_separator {
        return None;
    }
    Some((cursor, canonical))
}

/// The number form of ADR-0148, decision 3, if it starts at `start`.
///
/// Form B is `\d{2,}([.-]\d+)+`: at least two groups, the first of two digits
/// or more; the canonical token joins them with `-`, `.` and `-` being one and
/// the same separator.
fn number_at(text: &str, bytes: &[u8], start: usize) -> Option<(usize, String)> {
    if !starts_a_word(text, start) {
        return None;
    }
    let mut cursor = start;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    if cursor - start < 2 {
        return None;
    }
    let mut groups = 0;
    while matches!(bytes.get(cursor), Some(b'.' | b'-')) {
        let mut group_end = cursor + 1;
        while bytes.get(group_end).is_some_and(u8::is_ascii_digit) {
            group_end += 1;
        }
        if group_end == cursor + 1 {
            break;
        }
        cursor = group_end;
        groups += 1;
    }
    if groups == 0 || !ends_a_word(text, cursor) {
        return None;
    }
    let canonical = text[start..cursor]
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect();
    Some((cursor, canonical))
}

/// Whether `offset` starts a word: the text begins there, or the character
/// before it is not alphanumeric (ADR-0148, decision 3, "à une frontière de
/// mot").
fn starts_a_word(text: &str, offset: usize) -> bool {
    text[..offset]
        .chars()
        .next_back()
        .is_none_or(|previous| !previous.is_alphanumeric())
}

/// Whether a match ending at `offset` ends a word, the second half of the same
/// boundary rule.
fn ends_a_word(text: &str, offset: usize) -> bool {
    text[offset..]
        .chars()
        .next()
        .is_none_or(|next| !next.is_alphanumeric())
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
    use crible_core::config::{LEXICAL_LANGUAGES, LexicalIdentifiers};
    use crible_core::error::ErrorCode;
    use tantivy::tokenizer::Language;

    /// Tokens the chain of `code` produces for `text`, in order.
    fn tokens(code: &str, identifiers: LexicalIdentifiers, text: &str) -> Vec<String> {
        let mut analyzer = analyzer(code, identifiers).expect("the code is one of the table");
        let mut stream = analyzer.token_stream(text);
        let mut tokens = Vec::new();
        while let Some(token) = stream.next() {
            tokens.push(token.text.clone());
        }
        tokens
    }

    /// Tokens of the 1.0 chain, the one `identifiers = "none"` selects.
    fn none_tokens(code: &str, text: &str) -> Vec<String> {
        tokens(code, LexicalIdentifiers::None, text)
    }

    /// Text and position of each token of the chain, in stream order.
    fn positions(code: &str, identifiers: LexicalIdentifiers, text: &str) -> Vec<(String, usize)> {
        let mut analyzer = analyzer(code, identifiers).expect("the code is one of the table");
        let mut stream = analyzer.token_stream(text);
        let mut tokens = Vec::new();
        while let Some(token) = stream.next() {
            tokens.push((token.text.clone(), token.position));
        }
        tokens
    }

    // The canonical token of an identifier is the only token of the chain that
    // can hold a `-`: `SimpleTokenizer` splits on that character.
    fn canonical_tokens(tokens: &[String]) -> Vec<String> {
        tokens
            .iter()
            .filter(|token| token.contains('-'))
            .cloned()
            .collect()
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
            none_tokens("fr", "l'article 1240 du code civil"),
            ["articl", "1240", "cod", "civil"]
        );
    }

    /// ADR-0148, decisions 2 and 3: the chain of `identifiers = "v1"` appends the
    /// canonical token of an article, and keeps the tokens of the 1.0 chain. The
    /// test pins the whole list, as the ADR asks.
    ///
    /// Corrected by `@dev`: the test required `L1132-1` to analyse like
    /// `L. 1132-1`, which decision 1 forbids. `SimpleTokenizer` keeps the letter
    /// glued to the digits in the stored form (`l1132` is one piece, and not a
    /// stop word), and dropping it would be the replacement V2 of MES-11 did.
    /// Both forms still produce the one canonical token `l1132-1`, which is what
    /// decision 3 fixes.
    #[test]
    fn identifier_v1_appends_the_canonical_token_of_an_article() {
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "L. 1132-1"),
            ["1132", "1", "l1132-1"],
            "ADR-0148 fixes this list: the 1.0 tokens, then the canonical token"
        );
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "L1132-1"),
            ["l1132", "1", "l1132-1"],
            "the stored form keeps its 1.0 piece `l1132` and adds the same canonical token"
        );
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "R. 233-45"),
            ["r", "233", "45", "r233-45"],
            "the cited form keeps its pieces, `r` included, and adds the canonical token"
        );
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "R233-45"),
            ["r233", "45", "r233-45"],
            "the stored form keeps its piece `r233` and adds the same canonical token"
        );
    }

    /// ADR-0148, decision 3: the number form joins its groups with `-`, and `.`
    /// and `-` are one and the same separator.
    #[test]
    fn identifier_v1_appends_the_canonical_token_of_a_number() {
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "10-13.625"),
            ["10", "13", "625", "10-13-625"],
            "the criterion of the ticket: the pourvoi keeps a token of its own"
        );
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "81-70.489"),
            ["81", "70", "489", "81-70-489"]
        );
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "92-12345"),
            ["92", "12345", "92-12345"]
        );
        // ADR-0148, decision 4, "Cas limites à tester": a date matches the
        // number form, so it gains a canonical token too. The decision expects
        // no rank effect, but the token exists and the measurement counts it.
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "15-02-2011"),
            ["15", "02", "2011", "15-02-2011"],
            "a date is three digit groups: the canonical token is added"
        );
    }

    /// ADR-0148, decision 1: the canonical token is **added** to the tokens of
    /// the 1.0 chain, never put in their place. This is what separates the
    /// decision from V2 of MES-11, whose measured regression came from replacing
    /// them.
    #[test]
    fn identifier_v1_never_removes_a_one_zero_token() {
        for text in [
            "L. 1132-1",
            "L1132-1",
            "10-13.625",
            "81-70.489",
            "R. 233-45 rendu le 15-02-2011",
            "l'article 1240 du code civil",
            "SANP0422530D",
            "qu'il",
            "dommages-intérêts",
        ] {
            let none = none_tokens("fr", text);
            let v1 = tokens("fr", LexicalIdentifiers::V1, text);
            let kept: Vec<String> = v1
                .iter()
                .filter(|token| !token.contains('-'))
                .cloned()
                .collect();
            assert_eq!(
                kept, none,
                "every 1.0 token of {text:?} must survive, in the same order, under \"v1\""
            );
        }
    }

    /// ADR-0148, decision 3: the domain `v1` is closed. A value outside it gets
    /// exactly the tokens of the 1.0 chain, and no token is added.
    #[test]
    fn identifier_v1_adds_nothing_outside_the_domain() {
        for text in [
            "1240",
            "SANP0422530D",
            "ECLI:FR:CCASS:2011:C100123",
            "1.5",
            "1 240",
            "N°1234",
        ] {
            assert_eq!(
                tokens("fr", LexicalIdentifiers::V1, text),
                none_tokens("fr", text),
                "{text:?} is outside the v1 domain: no canonical token may be added"
            );
        }
        assert_eq!(
            tokens("fr", LexicalIdentifiers::V1, "1240"),
            ["1240"],
            "a bare number is already one token (ADR-0148, decision 1)"
        );
    }

    /// ADR-0148, decision 4: an identifier of 40 bytes or more leaves no token,
    /// the pieces and the canonical token included (`RemoveLongFilter(40)`).
    #[test]
    fn a_long_identifier_is_dropped_from_both_sides() {
        // 40 digits: the piece alone already reaches the limit of the filter,
        // and the canonical token is longer still.
        let text = "L. 1234567890123456789012345678901234567890-1";
        let none = none_tokens("fr", text);
        let v1 = tokens("fr", LexicalIdentifiers::V1, text);
        assert_eq!(
            v1, none,
            "the canonical token must be dropped like the piece it doubles"
        );
        assert!(
            v1.iter().all(|token| token.len() < 40),
            "RemoveLongFilter(40) keeps no token of 40 bytes or more: {v1:?}"
        );
        assert!(
            canonical_tokens(&v1).is_empty(),
            "no canonical token may survive the filter: {v1:?}"
        );
    }

    /// ADR-0148, decision 4: the canonical token carries the position of the
    /// first piece of the identifier and increments nothing, so the positions of
    /// the tokens after it do not move.
    #[test]
    fn identifier_v1_keeps_the_position_of_the_first_piece() {
        let none = positions("fr", LexicalIdentifiers::None, "avant L. 1132-1 apres");
        let v1 = positions("fr", LexicalIdentifiers::V1, "avant L. 1132-1 apres");
        assert_eq!(
            v1,
            vec![
                ("avant".to_owned(), 0),
                ("1132".to_owned(), 2),
                ("1".to_owned(), 3),
                // The `l` piece sits at position 1 and is a French stop word; the
                // canonical token takes its position.
                ("l1132-1".to_owned(), 1),
                // The French stemmer reduces `apres` to `apre`, as it does in
                // 1.0: the test pins a position, not a stem.
                ("apre".to_owned(), 4),
            ],
            "the canonical token is the synonym of the whole span, on one position"
        );
        let kept: Vec<(String, usize)> = v1
            .iter()
            .filter(|(token, _)| !token.contains('-'))
            .cloned()
            .collect();
        assert_eq!(
            kept, none,
            "the tokens after the identifier must keep the positions of 1.0"
        );

        let v1 = positions("fr", LexicalIdentifiers::V1, "avant 10-13.625 apres");
        assert_eq!(
            v1,
            vec![
                ("avant".to_owned(), 0),
                ("10".to_owned(), 1),
                ("13".to_owned(), 2),
                ("625".to_owned(), 3),
                ("10-13-625".to_owned(), 1),
                ("apre".to_owned(), 4),
            ],
            "the canonical number takes the position of its first group"
        );
    }

    /// ADR-0148, decision 4: a repeated identifier produces the canonical token
    /// as often as it appears, so its `tf` counts both occurrences.
    #[test]
    fn identifier_v1_repeats_the_canonical_token_of_a_repeated_identifier() {
        let tokens = tokens("fr", LexicalIdentifiers::V1, "10-13.625 puis 10-13.625");
        let canonical: Vec<&String> = tokens
            .iter()
            .filter(|token| token.as_str() == "10-13-625")
            .collect();
        assert_eq!(
            canonical.len(),
            2,
            "a repeated identifier must give the canonical token twice: {tokens:?}"
        );
    }

    /// ADR-0148, decision 5: the absence of the setting is the 1.0 chain, and
    /// the 1.0 chain is exactly what `docs/03` documented before the ADR.
    #[test]
    fn identifiers_none_reproduces_the_one_zero_chain() {
        assert_eq!(none_tokens("fr", "L. 1132-1"), ["1132", "1"]);
        // Corrected by `@dev`: the stored form keeps the letter glued to the
        // digits, and `l1132` is not a stop word, so the piece survives.
        assert_eq!(none_tokens("fr", "L1132-1"), ["l1132", "1"]);
        assert_eq!(none_tokens("fr", "10-13.625"), ["10", "13", "625"]);
        assert_eq!(none_tokens("fr", "81-70.489"), ["81", "70", "489"]);
        for text in ["1240", "SANP0422530D", "ECLI:FR:CCASS:2011:C100123"] {
            assert!(
                canonical_tokens(&none_tokens("fr", text)).is_empty(),
                "{text:?} must produce no canonical token under \"none\""
            );
        }
    }

    /// Both halves of the elision are French stop words, so the whole phrase
    /// analyses to nothing (MES-11, Q-04).
    #[test]
    fn french_elision_tokens() {
        assert!(none_tokens("fr", "qu'il").is_empty());
    }

    /// The compound is split and never joined back (MES-11, Q-04).
    #[test]
    fn french_compound_tokens() {
        assert_eq!(
            none_tokens("fr", "dommages-intérêts"),
            ["dommag", "interet"]
        );
    }

    /// ADR-0086: `ar`, `el`, `ro`, `ta` and `tr` have no stop-word list, so
    /// their tokens are those of a chain without that filter: French stop words
    /// survive in every one of them.
    #[test]
    fn a_language_without_a_stop_word_list_keeps_every_token() {
        for code in ["ar", "el", "ro", "ta", "tr"] {
            assert_eq!(
                none_tokens(code, "de la"),
                ["de", "la"],
                "{code} has no stop-word list, so the French stop words must survive"
            );
        }
        assert_eq!(none_tokens("tr", "qu'il"), ["qu", "il"]);
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
        let error = match analyzer("FR", LexicalIdentifiers::None) {
            Ok(_) => panic!("FR is not a code of the table"),
            Err(error) => error,
        };
        assert_eq!(error.code(), ErrorCode::Internal);
        assert!(error.message().contains("FR"), "{}", error.message());
    }
}
