// SPDX-License-Identifier: Apache-2.0

//! Index configuration: schema, search settings, and the frozen/replaceable
//! split enforced by `apply_patch`.
//!
//! References: `docs/04-donnees-et-ingestion.md`, section "Configuration d'un
//! index" and "Ce qui est figé, ce qui est réglable"; ADR-0030; ADR-0031;
//! ADR-0054 (bounds of the acceptance settings); ADR-0074 (the exact threshold
//! is optional at creation and materialised); INV-16
//! (`docs/08-invariants-et-tests.md`).

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::error::{CribleError, ErrorCode};
use crate::filter::Operator;
use crate::json;

/// Type of a schema field, as documented in `docs/04`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// Opaque string, compared and grouped by exact value.
    Keyword,
    /// Signed integer.
    Integer,
    /// Floating-point number.
    Float,
    /// Calendar date written `YYYY-MM-DD`.
    Date,
    /// Boolean.
    Bool,
}

impl FieldType {
    /// Every documented field type, for exhaustive checks.
    pub const ALL: [Self; 5] = [
        Self::Keyword,
        Self::Integer,
        Self::Float,
        Self::Date,
        Self::Bool,
    ];

    /// Wire name used in the configuration and in error messages.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::Date => "date",
            Self::Bool => "bool",
        }
    }

    /// Operators allowed on a field of this type (`docs/05-api.md`).
    #[must_use]
    pub fn operators(self) -> &'static [Operator] {
        use Operator::{Eq, Exists, Gt, Gte, In, Lt, Lte, Ne, Nin};
        match self {
            Self::Keyword => &[Eq, Ne, In, Nin, Exists],
            Self::Integer | Self::Float | Self::Date => &[Eq, Ne, Lt, Lte, Gt, Gte, In, Exists],
            Self::Bool => &[Eq, Exists],
        }
    }

    fn parse(value: &Value, path: &str) -> Result<Self, CribleError> {
        let raw = json::string(value, path)?;
        match raw.as_str() {
            "keyword" => Ok(Self::Keyword),
            "integer" => Ok(Self::Integer),
            "float" => Ok(Self::Float),
            "date" => Ok(Self::Date),
            "bool" => Ok(Self::Bool),
            other => Err(json::invalid(format!(
                "unknown field type '{other}' at {path}; expected one of keyword, integer, float, date, bool"
            ))),
        }
    }
}

/// Level at which a field is declared (`docs/04`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldLevel {
    /// Field carried by the document and inherited by all its passages.
    Document,
    /// Field carried by a single passage.
    Passage,
}

impl FieldLevel {
    /// Name of the schema object holding the fields of this level.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document_fields",
            Self::Passage => "passage_fields",
        }
    }
}

/// One declared field, with its type and its two options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldSpec {
    /// Type of the values.
    pub field_type: FieldType,
    /// Whether the field may appear in a filter.
    pub filterable: bool,
    /// Whether the field is returned in search results.
    pub returned: bool,
}

impl FieldSpec {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["type", "filterable", "returned"])?;
        let field_type =
            FieldType::parse(json::member(object, path, "type")?, &format!("{path}.type"))?;
        let filterable = match object.get("filterable") {
            Some(value) => json::boolean(value, &format!("{path}.filterable"))?,
            None => true,
        };
        let returned = match object.get("returned") {
            Some(value) => json::boolean(value, &format!("{path}.returned"))?,
            None => true,
        };
        Ok(Self {
            field_type,
            filterable,
            returned,
        })
    }

    /// Writes the field in the JSON form of `docs/04`, with both options
    /// explicit.
    fn to_value(&self) -> Value {
        serde_json::json!({
            "type": self.field_type.as_str(),
            "filterable": self.filterable,
            "returned": self.returned,
        })
    }
}

/// Schema of an index: fields declared at document level and at passage level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    document_fields: BTreeMap<String, FieldSpec>,
    passage_fields: BTreeMap<String, FieldSpec>,
}

impl Schema {
    /// Parses a schema from its JSON form.
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid schema JSON: {error}")))?;
        Self::from_value(&value, "$")
    }

    /// Parses a schema from an already-decoded JSON value.
    pub fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["document_fields", "passage_fields"])?;
        let document_fields = fields_from_value(
            json::member(object, path, "document_fields")?,
            &format!("{path}.document_fields"),
        )?;
        let passage_fields = fields_from_value(
            json::member(object, path, "passage_fields")?,
            &format!("{path}.passage_fields"),
        )?;
        let schema = Self {
            document_fields,
            passage_fields,
        };
        schema.validate()?;
        Ok(schema)
    }

    /// Looks a field up by name, whatever its level.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<(&FieldSpec, FieldLevel)> {
        if let Some(spec) = self.document_fields.get(name) {
            Some((spec, FieldLevel::Document))
        } else {
            self.passage_fields
                .get(name)
                .map(|spec| (spec, FieldLevel::Passage))
        }
    }

    /// Every declared field with its level, document fields first and each
    /// group sorted by name.
    ///
    /// The batch reader walks this list to check the columns of a Parquet file
    /// against the schema of the index.
    pub fn fields(&self) -> impl Iterator<Item = (&str, FieldLevel, &FieldSpec)> {
        self.document_fields
            .iter()
            .map(|(name, spec)| (name.as_str(), FieldLevel::Document, spec))
            .chain(
                self.passage_fields
                    .iter()
                    .map(|(name, spec)| (name.as_str(), FieldLevel::Passage, spec)),
            )
    }

    fn spec_at(&self, level: FieldLevel, name: &str) -> Option<&FieldSpec> {
        self.map(level).get(name)
    }

    fn spec_at_mut(&mut self, level: FieldLevel, name: &str) -> Option<&mut FieldSpec> {
        self.map_mut(level).get_mut(name)
    }

    fn map(&self, level: FieldLevel) -> &BTreeMap<String, FieldSpec> {
        match level {
            FieldLevel::Document => &self.document_fields,
            FieldLevel::Passage => &self.passage_fields,
        }
    }

    fn map_mut(&mut self, level: FieldLevel) -> &mut BTreeMap<String, FieldSpec> {
        match level {
            FieldLevel::Document => &mut self.document_fields,
            FieldLevel::Passage => &mut self.passage_fields,
        }
    }

    /// Every declared field name, document level first.
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.document_fields
            .keys()
            .chain(self.passage_fields.keys())
            .map(String::as_str)
    }

    /// Serializes the schema in the JSON form of `docs/04`.
    ///
    /// The two options are written explicitly, so the text round-trips through
    /// [`Schema::from_json_str`] even when a caller relied on the defaults. The
    /// keys are sorted by name, as [`Schema::fields`] yields them.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut document_fields = Map::new();
        let mut passage_fields = Map::new();
        for (name, level, spec) in self.fields() {
            let target = match level {
                FieldLevel::Document => &mut document_fields,
                FieldLevel::Passage => &mut passage_fields,
            };
            target.insert(name.to_owned(), spec.to_value());
        }
        Value::Object(Map::from_iter([
            ("document_fields".to_owned(), Value::Object(document_fields)),
            ("passage_fields".to_owned(), Value::Object(passage_fields)),
        ]))
        .to_string()
    }

    /// Refuses a name declared at both levels (ADR-0030).
    fn validate(&self) -> Result<(), CribleError> {
        for name in self.document_fields.keys() {
            if self.passage_fields.contains_key(name) {
                return Err(json::invalid(format!(
                    "field '{name}' is declared in both document_fields and passage_fields; a field name must designate one level"
                )));
            }
        }
        Ok(())
    }
}

fn fields_from_value(
    value: &Value,
    path: &str,
) -> Result<BTreeMap<String, FieldSpec>, CribleError> {
    let object = json::object(value, path)?;
    let mut fields = BTreeMap::new();
    for (name, spec) in object {
        fields.insert(
            name.clone(),
            FieldSpec::from_value(spec, &format!("{path}.{name}"))?,
        );
    }
    Ok(fields)
}

/// Precision of the reference vectors, frozen at index creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferencePrecision {
    /// Half precision.
    Fp16,
    /// Single precision.
    Fp32,
}

impl ReferencePrecision {
    fn parse(value: &Value, path: &str) -> Result<Self, CribleError> {
        let raw = json::string(value, path)?;
        match raw.as_str() {
            "fp16" => Ok(Self::Fp16),
            "fp32" => Ok(Self::Fp32),
            other => Err(json::invalid(format!(
                "unknown reference_precision '{other}' at {path}; expected fp16 or fp32"
            ))),
        }
    }
}

/// Scan strategy of the dense branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStrategy {
    /// Binary-code scan followed by exact rescoring.
    Binary,
    /// Always exact.
    None,
}

impl ScanStrategy {
    fn parse(value: &Value, path: &str) -> Result<Self, CribleError> {
        let raw = json::string(value, path)?;
        match raw.as_str() {
            "binary" => Ok(Self::Binary),
            "none" => Ok(Self::None),
            other => Err(json::invalid(format!(
                "unknown scan '{other}' at {path}; expected binary or none"
            ))),
        }
    }
}

/// How the dense and lexical rankings are combined (`docs/03`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FusionConfig {
    /// Reciprocal rank fusion.
    Rrf,
    /// Relative score with a weight.
    RelativeScore {
        /// Weight given to the dense branch, between 0 and 1.
        alpha: f64,
    },
}

impl FusionConfig {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["method", "alpha"])?;
        let method = json::string_member(object, path, "method")?;
        match method.as_str() {
            "rrf" => {
                if object.contains_key("alpha") {
                    return Err(json::invalid(format!(
                        "alpha is not allowed with rrf at {path}.alpha"
                    )));
                }
                Ok(Self::Rrf)
            }
            "relative_score" => {
                let alpha = json::float_member(object, path, "alpha")?;
                Ok(Self::RelativeScore { alpha })
            }
            other => Err(json::invalid(format!(
                "unknown fusion method '{other}' at {path}.method; expected rrf or relative_score"
            ))),
        }
    }
}

/// Lexical branch settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexicalConfig {
    /// Whether the lexical branch is built and queried.
    pub enabled: bool,
    /// Language of the analyzer.
    pub language: String,
    /// Whether term positions are stored.
    pub positions: bool,
}

impl LexicalConfig {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["enabled", "language", "positions"])?;
        Ok(Self {
            enabled: json::boolean_member(object, path, "enabled")?,
            language: json::string_member(object, path, "language")?,
            positions: json::boolean_member(object, path, "positions")?,
        })
    }
}

/// Dense search settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchConfig {
    /// Scan strategy of the dense branch.
    pub scan: ScanStrategy,
    /// Number of candidates re-scored exactly.
    pub rescore_candidates: u32,
    /// Pool size up to which (inclusive) the search is exact.
    ///
    /// Optional in `index.json`: absent, it takes the value of
    /// `rescore_candidates` at the creation of the index, and the creation
    /// writes that value out (ADR-0074, decision 2). A later `PATCH` of
    /// `rescore_candidates` therefore does not move it.
    pub exact_threshold: u32,
    /// Candidates kept by each branch before fusion.
    pub candidates_per_branch: u32,
    /// Fusion of the two rankings.
    pub fusion: FusionConfig,
}

impl SearchConfig {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(
            object,
            path,
            &[
                "scan",
                "rescore_candidates",
                "exact_threshold",
                "candidates_per_branch",
                "fusion",
            ],
        )?;
        let rescore_candidates = json::u32_member(object, path, "rescore_candidates")?;
        Ok(Self {
            scan: ScanStrategy::parse(
                json::member(object, path, "scan")?,
                &format!("{path}.scan"),
            )?,
            rescore_candidates,
            // ADR-0074, decision 2: absent, the threshold takes the value of N
            // at the creation of the index. The creation writes that value into
            // `index.json`, so a stored configuration always names it.
            exact_threshold: optional_u32(object, path, "exact_threshold")?
                .unwrap_or(rescore_candidates),
            candidates_per_branch: json::u32_member(object, path, "candidates_per_branch")?,
            fusion: FusionConfig::from_value(
                json::member(object, path, "fusion")?,
                &format!("{path}.fusion"),
            )?,
        })
    }
}

/// Ingestion settings, including the acceptance check.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestConfig {
    /// Number of passages re-encoded during the acceptance check.
    pub acceptance_sample: u32,
    /// Minimum cosine similarity accepted for the sample.
    pub acceptance_min_cosine: f64,
}

impl IngestConfig {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(
            object,
            path,
            &["acceptance_sample", "acceptance_min_cosine"],
        )?;
        let ingest = Self {
            acceptance_sample: json::u32_member(object, path, "acceptance_sample")?,
            acceptance_min_cosine: json::float_member(object, path, "acceptance_min_cosine")?,
        };
        ingest.validate(path)?;
        Ok(ingest)
    }

    /// Refuses the two acceptance settings outside their domain (ADR-0054).
    ///
    /// A sample of zero would silently disable control 9 of `docs/04`, and a
    /// threshold outside `]0, 1]` cannot express "close enough"; the comparison
    /// is written so that NaN is outside the domain too. The default of
    /// `acceptance_min_cosine` (0,999, ADR-0052) is unchanged.
    fn validate(&self, path: &str) -> Result<(), CribleError> {
        if self.acceptance_sample == 0 {
            return Err(json::invalid(format!(
                "setting 'acceptance_sample' must be at least 1 at \
                 {path}.acceptance_sample; zero would disable the acceptance check \
                 (ADR-0054)"
            )));
        }
        if !(self.acceptance_min_cosine > 0.0 && self.acceptance_min_cosine <= 1.0) {
            return Err(json::invalid(format!(
                "setting 'acceptance_min_cosine' must be in ]0, 1] at \
                 {path}.acceptance_min_cosine, got {} (ADR-0054)",
                self.acceptance_min_cosine
            )));
        }
        Ok(())
    }
}

/// Compaction settings.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionConfig {
    /// Fraction of dead passages that triggers a compaction.
    pub dead_ratio: f64,
}

impl CompactionConfig {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["dead_ratio"])?;
        Ok(Self {
            dead_ratio: json::float_member(object, path, "dead_ratio")?,
        })
    }
}

/// Configuration of one index, as stored in `index.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexConfig {
    /// Index name, frozen.
    pub name: String,
    /// Name of the model directory, frozen.
    pub model: String,
    /// Reference vector precision, frozen.
    pub reference_precision: ReferencePrecision,
    /// Schema of the index.
    pub schema: Schema,
    /// Fields driving the clustering, applied at the next compaction.
    pub cluster_by: Vec<String>,
    /// Lexical branch settings.
    pub lexical: LexicalConfig,
    /// Dense search settings.
    pub search: SearchConfig,
    /// Ingestion settings.
    pub ingest: IngestConfig,
    /// Compaction settings.
    pub compaction: CompactionConfig,
}

const TOP_LEVEL_KEYS: [&str; 9] = [
    "name",
    "model",
    "reference_precision",
    "schema",
    "cluster_by",
    "lexical",
    "search",
    "ingest",
    "compaction",
];

impl IndexConfig {
    /// Parses and validates an index configuration from its JSON form.
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid configuration JSON: {error}")))?;
        Self::from_value(&value)
    }

    /// Parses and validates an index configuration from a decoded JSON value.
    pub fn from_value(value: &Value) -> Result<Self, CribleError> {
        let object = json::object(value, "$")?;
        json::reject_unknown_keys(object, "$", &TOP_LEVEL_KEYS)?;
        let name = json::string_member(object, "$", "name")?;
        if name.is_empty() {
            return Err(json::invalid("setting 'name' must not be empty at $.name"));
        }
        let model = json::string_member(object, "$", "model")?;
        if model.is_empty() {
            return Err(json::invalid(
                "setting 'model' must not be empty at $.model",
            ));
        }
        Ok(Self {
            name,
            model,
            reference_precision: ReferencePrecision::parse(
                json::member(object, "$", "reference_precision")?,
                "$.reference_precision",
            )?,
            schema: Schema::from_value(json::member(object, "$", "schema")?, "$.schema")?,
            cluster_by: string_array(json::member(object, "$", "cluster_by")?, "$.cluster_by")?,
            lexical: LexicalConfig::from_value(json::member(object, "$", "lexical")?, "$.lexical")?,
            search: SearchConfig::from_value(json::member(object, "$", "search")?, "$.search")?,
            ingest: IngestConfig::from_value(json::member(object, "$", "ingest")?, "$.ingest")?,
            compaction: CompactionConfig::from_value(
                json::member(object, "$", "compaction")?,
                "$.compaction",
            )?,
        })
    }

    /// Returns the stored form of a configuration: every setting that has a
    /// default is written out, so `index.json` is self-descriptive.
    ///
    /// `search.exact_threshold` is the only such setting today (ADR-0074,
    /// decision 2): absent, it takes the value of `search.rescore_candidates`,
    /// and the creation writes that value into the file, so a later `PATCH` of
    /// N cannot move the stored threshold. A configuration that already names
    /// the key is returned **verbatim**, so a creation never reformats a file
    /// it does not have to complete.
    ///
    /// When the key is missing, the file is **edited**, not rewritten: the
    /// member is inserted into the `search` object of the raw text, and every
    /// other byte of the file — key order, indentation, spacing — stays exactly
    /// as the operator wrote it. Re-serialising a decoded value would sort the
    /// keys and normalise the indentation (`serde_json` without
    /// `preserve_order`), which would rewrite a file for one missing key.
    pub fn materialised_json(raw: &str) -> Result<String, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid configuration JSON: {error}")))?;
        let config = Self::from_value(&value)?;
        let named = value
            .get("search")
            .and_then(Value::as_object)
            .is_some_and(|search| search.contains_key("exact_threshold"));
        if named {
            return Ok(raw.to_owned());
        }
        let threshold = config.search.exact_threshold;
        let completed = insert_search_threshold(raw, threshold)?;
        // The insertion is textual, so it is checked: a bug in the scan must
        // never leave an `index.json` that no later open can parse, and the key
        // it adds must land under `$.search.exact_threshold`, with no other
        // change of the decoded configuration.
        let completed_value: Value = serde_json::from_str(&completed).map_err(|error| {
            CribleError::new(
                ErrorCode::Internal,
                format!("the materialised configuration is not JSON: {error}"),
            )
        })?;
        let mut expected = value;
        let search = expected
            .get_mut("search")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                CribleError::new(
                    ErrorCode::Internal,
                    "the decoded configuration has no object at $.search".to_owned(),
                )
            })?;
        search.insert("exact_threshold".to_owned(), Value::from(threshold));
        if completed_value != expected {
            return Err(CribleError::new(
                ErrorCode::Internal,
                "the materialised configuration does not carry $.search.exact_threshold alone"
                    .to_owned(),
            ));
        }
        Ok(completed)
    }
}

/// Inserts `search.exact_threshold` in the raw text of a configuration.
///
/// The raw text is already known to be valid JSON and to hold an object at
/// `$.search`; this function only finds the byte range of that object and adds
/// the member after its opening brace, in the indentation of the object. It
/// never decodes and re-encodes, so the rest of the file keeps its bytes.
fn insert_search_threshold(raw: &str, value: u32) -> Result<String, CribleError> {
    let (open, close) = top_level_object_range(raw, "search").ok_or_else(|| {
        CribleError::new(
            ErrorCode::Internal,
            "cannot locate the object at $.search in the configuration text".to_owned(),
        )
    })?;
    let inner = &raw[open + 1..close];
    let member = format!("\"exact_threshold\": {value}");
    let insertion = if inner.contains('\n') {
        // Follow the file's line ending: a CRLF configuration keeps CRLF on
        // the inserted line, so the edit does not mix the two conventions.
        let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
        let close_indent = line_indent(raw, close);
        let indent = member_indent(inner, &close_indent);
        let comma = if inner.trim().is_empty() { "" } else { "," };
        format!("{newline}{indent}{member}{comma}")
    } else if inner.trim().is_empty() {
        member
    } else {
        format!("{member},")
    };
    let mut completed = String::with_capacity(raw.len() + insertion.len());
    completed.push_str(&raw[..open + 1]);
    completed.push_str(&insertion);
    completed.push_str(&raw[open + 1..]);
    Ok(completed)
}

/// Byte range of the object value of the top-level member `key`, braces
/// included, in a text already known to be a JSON object.
///
/// The scan skips string contents, so a key named `search` inside a value never
/// matches. `None` means the member or its object is absent, which the caller
/// treats as a contradiction with the decoded value.
fn top_level_object_range(raw: &str, key: &str) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();
    let mut depth = 0_usize;
    let mut index = 0_usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index;
                index = string_end(bytes, start)?;
                if depth == 1 {
                    let token = &raw[start + 1..index - 1];
                    let mut cursor = index;
                    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                        cursor += 1;
                    }
                    if cursor < bytes.len()
                        && bytes[cursor] == b':'
                        && !token.contains('\\')
                        && token == key
                    {
                        let mut open = cursor + 1;
                        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
                            open += 1;
                        }
                        if bytes.get(open) != Some(&b'{') {
                            return None;
                        }
                        return Some((open, matching_close(bytes, open)?));
                    }
                }
                // `string_end` already points past the closing quote: the byte
                // it lands on may be `}`, `]` or `,` and must still be scanned.
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }
    None
}

/// Index just past the closing quote of the string that starts at `start`.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// Index of the `}` that closes the object opened at `open`.
fn matching_close(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = string_end(bytes, index)?;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Leading whitespace of the line that holds `position`, which must be the
/// first non-whitespace character of that line; empty when it is not.
fn line_indent(raw: &str, position: usize) -> String {
    let line_start = raw[..position].rfind('\n').map_or(0, |offset| offset + 1);
    let line = &raw[line_start..position];
    if line
        .chars()
        .all(|character| character == ' ' || character == '\t')
    {
        line.to_owned()
    } else {
        String::new()
    }
}

/// Indentation of one member of a pretty-printed object: the indentation of its
/// first member, or one level below the closing brace for an empty object.
fn member_indent(inner: &str, close_indent: &str) -> String {
    if let Some(first) = inner.find(|character: char| !character.is_whitespace()) {
        let line_start = inner[..first].rfind('\n').map_or(0, |offset| offset + 1);
        let indent = &inner[line_start..first];
        if !indent.is_empty() {
            return indent.to_owned();
        }
    }
    format!("{close_indent}  ")
}

fn string_array(value: &Value, path: &str) -> Result<Vec<String>, CribleError> {
    let items = value.as_array().ok_or_else(|| {
        json::invalid(format!(
            "expected an array of strings at {path}, got {}",
            json::json_type(value)
        ))
    })?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| json::string(item, &format!("{path}[{index}]")))
        .collect()
}

/// A `PATCH` body for an index configuration.
///
/// Parsing rejects unknown keys at every level (ADR-0031); [`apply_patch`]
/// rejects changes to frozen settings (INV-16).
#[derive(Debug, Clone, Default)]
pub struct IndexPatch {
    name: Option<String>,
    model: Option<String>,
    reference_precision: Option<ReferencePrecision>,
    schema: Option<SchemaPatch>,
    cluster_by: Option<Vec<String>>,
    lexical: Option<LexicalPatch>,
    search: Option<SearchPatch>,
    ingest: Option<IngestPatch>,
    compaction: Option<CompactionPatch>,
}

impl IndexPatch {
    /// Parses and validates a patch body from its JSON form.
    pub fn from_json_str(raw: &str) -> Result<Self, CribleError> {
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| json::invalid(format!("invalid patch JSON: {error}")))?;
        Self::from_value(&value)
    }

    /// Parses and validates a patch body from a decoded JSON value.
    pub fn from_value(value: &Value) -> Result<Self, CribleError> {
        let object = json::object(value, "$")?;
        json::reject_unknown_keys(object, "$", &TOP_LEVEL_KEYS)?;
        Ok(Self {
            name: optional_string(object, "$", "name")?,
            model: optional_string(object, "$", "model")?,
            reference_precision: match object.get("reference_precision") {
                Some(value) => Some(ReferencePrecision::parse(value, "$.reference_precision")?),
                None => None,
            },
            schema: match object.get("schema") {
                Some(value) => Some(SchemaPatch::from_value(value, "$.schema")?),
                None => None,
            },
            cluster_by: match object.get("cluster_by") {
                Some(value) => Some(string_array(value, "$.cluster_by")?),
                None => None,
            },
            lexical: optional_patch(object, "lexical", "$.lexical", LexicalPatch::from_value)?,
            search: optional_patch(object, "search", "$.search", SearchPatch::from_value)?,
            ingest: optional_patch(object, "ingest", "$.ingest", IngestPatch::from_value)?,
            compaction: optional_patch(
                object,
                "compaction",
                "$.compaction",
                CompactionPatch::from_value,
            )?,
        })
    }
}

fn optional_string(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<Option<String>, CribleError> {
    match object.get(key) {
        Some(value) => json::string(value, &format!("{path}.{key}")).map(Some),
        None => Ok(None),
    }
}

fn optional_patch<T>(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    parse: fn(&Value, &str) -> Result<T, CribleError>,
) -> Result<Option<T>, CribleError> {
    match object.get(key) {
        Some(value) => parse(value, path).map(Some),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Default)]
struct SchemaPatch {
    document_fields: BTreeMap<String, FieldPatch>,
    passage_fields: BTreeMap<String, FieldPatch>,
}

impl SchemaPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["document_fields", "passage_fields"])?;
        Ok(Self {
            document_fields: match object.get("document_fields") {
                Some(value) => field_patches(value, &format!("{path}.document_fields"))?,
                None => BTreeMap::new(),
            },
            passage_fields: match object.get("passage_fields") {
                Some(value) => field_patches(value, &format!("{path}.passage_fields"))?,
                None => BTreeMap::new(),
            },
        })
    }

    fn reject_immutable(&self, config: &IndexConfig) -> Result<(), CribleError> {
        for (level, fields) in [
            (FieldLevel::Document, &self.document_fields),
            (FieldLevel::Passage, &self.passage_fields),
        ] {
            for (name, patch) in fields {
                let path = format!("$.schema.{}.{name}", level.as_str());
                if let Some(existing) = config.schema.spec_at(level, name)
                    && let Some(field_type) = patch.field_type
                    && field_type != existing.field_type
                {
                    return Err(CribleError::new(
                        ErrorCode::ImmutableSetting,
                        format!(
                            "field '{name}' type is immutable at {path}.type; the type of an existing field cannot change"
                        ),
                    ));
                }
                // Declaring the name at the other level is not a move: the
                // resulting schema would hold it twice, which ADR-0030 refuses
                // with `invalid_request` during validation (see `apply_patch`).
            }
        }
        Ok(())
    }

    fn apply(&self, schema: &mut Schema) -> Result<(), CribleError> {
        for (level, fields) in [
            (FieldLevel::Document, &self.document_fields),
            (FieldLevel::Passage, &self.passage_fields),
        ] {
            for (name, patch) in fields {
                patch.apply(name, level, schema)?;
            }
        }
        Ok(())
    }
}

fn field_patches(value: &Value, path: &str) -> Result<BTreeMap<String, FieldPatch>, CribleError> {
    let object = json::object(value, path)?;
    let mut patches = BTreeMap::new();
    for (name, patch) in object {
        patches.insert(
            name.clone(),
            FieldPatch::from_value(patch, &format!("{path}.{name}"))?,
        );
    }
    Ok(patches)
}

#[derive(Debug, Clone, Default)]
struct FieldPatch {
    field_type: Option<FieldType>,
    filterable: Option<bool>,
    returned: Option<bool>,
}

impl FieldPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["type", "filterable", "returned"])?;
        Ok(Self {
            field_type: match object.get("type") {
                Some(value) => Some(FieldType::parse(value, &format!("{path}.type"))?),
                None => None,
            },
            filterable: match object.get("filterable") {
                Some(value) => Some(json::boolean(value, &format!("{path}.filterable"))?),
                None => None,
            },
            returned: match object.get("returned") {
                Some(value) => Some(json::boolean(value, &format!("{path}.returned"))?),
                None => None,
            },
        })
    }

    fn apply(&self, name: &str, level: FieldLevel, schema: &mut Schema) -> Result<(), CribleError> {
        let path = format!("$.schema.{}.{name}", level.as_str());
        if let Some(existing) = schema.spec_at_mut(level, name) {
            if let Some(field_type) = self.field_type {
                existing.field_type = field_type;
            }
            if let Some(filterable) = self.filterable {
                existing.filterable = filterable;
            }
            if let Some(returned) = self.returned {
                existing.returned = returned;
            }
            return Ok(());
        }
        if let Some((_, declared_at)) = schema.field(name) {
            return Err(json::invalid(format!(
                "field '{name}' is already declared in {} and cannot also be declared in {}; a field name must designate one level",
                declared_at.as_str(),
                level.as_str()
            )));
        }
        let Some(field_type) = self.field_type else {
            return Err(json::invalid(format!(
                "new field '{name}' at {path} has no type; a 'type' is required to add a field"
            )));
        };
        schema.map_mut(level).insert(
            name.to_owned(),
            FieldSpec {
                field_type,
                filterable: self.filterable.unwrap_or(true),
                returned: self.returned.unwrap_or(true),
            },
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
struct LexicalPatch {
    enabled: Option<bool>,
    language: Option<String>,
    positions: Option<bool>,
}

impl LexicalPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["enabled", "language", "positions"])?;
        Ok(Self {
            enabled: match object.get("enabled") {
                Some(value) => Some(json::boolean(value, &format!("{path}.enabled"))?),
                None => None,
            },
            language: optional_string(object, path, "language")?,
            positions: match object.get("positions") {
                Some(value) => Some(json::boolean(value, &format!("{path}.positions"))?),
                None => None,
            },
        })
    }

    fn apply(&self, config: &mut LexicalConfig) {
        if let Some(enabled) = self.enabled {
            config.enabled = enabled;
        }
        if let Some(language) = &self.language {
            config.language.clone_from(language);
        }
        if let Some(positions) = self.positions {
            config.positions = positions;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SearchPatch {
    scan: Option<ScanStrategy>,
    rescore_candidates: Option<u32>,
    exact_threshold: Option<u32>,
    candidates_per_branch: Option<u32>,
    fusion: Option<FusionConfig>,
}

impl SearchPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(
            object,
            path,
            &[
                "scan",
                "rescore_candidates",
                "exact_threshold",
                "candidates_per_branch",
                "fusion",
            ],
        )?;
        Ok(Self {
            scan: match object.get("scan") {
                Some(value) => Some(ScanStrategy::parse(value, &format!("{path}.scan"))?),
                None => None,
            },
            rescore_candidates: optional_u32(object, path, "rescore_candidates")?,
            exact_threshold: optional_u32(object, path, "exact_threshold")?,
            candidates_per_branch: optional_u32(object, path, "candidates_per_branch")?,
            fusion: match object.get("fusion") {
                Some(value) => Some(FusionConfig::from_value(value, &format!("{path}.fusion"))?),
                None => None,
            },
        })
    }

    fn apply(&self, config: &mut SearchConfig) {
        if let Some(scan) = self.scan {
            config.scan = scan;
        }
        if let Some(rescore_candidates) = self.rescore_candidates {
            config.rescore_candidates = rescore_candidates;
        }
        if let Some(exact_threshold) = self.exact_threshold {
            config.exact_threshold = exact_threshold;
        }
        if let Some(candidates_per_branch) = self.candidates_per_branch {
            config.candidates_per_branch = candidates_per_branch;
        }
        if let Some(fusion) = self.fusion {
            config.fusion = fusion;
        }
    }
}

fn optional_u32(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<Option<u32>, CribleError> {
    match object.get(key) {
        Some(value) => json::u32_value(value, &format!("{path}.{key}")).map(Some),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Default)]
struct IngestPatch {
    acceptance_sample: Option<u32>,
    acceptance_min_cosine: Option<f64>,
}

impl IngestPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(
            object,
            path,
            &["acceptance_sample", "acceptance_min_cosine"],
        )?;
        Ok(Self {
            acceptance_sample: optional_u32(object, path, "acceptance_sample")?,
            acceptance_min_cosine: match object.get("acceptance_min_cosine") {
                Some(value) => Some(json::float(
                    value,
                    &format!("{path}.acceptance_min_cosine"),
                )?),
                None => None,
            },
        })
    }

    fn apply(&self, config: &mut IngestConfig) {
        if let Some(acceptance_sample) = self.acceptance_sample {
            config.acceptance_sample = acceptance_sample;
        }
        if let Some(acceptance_min_cosine) = self.acceptance_min_cosine {
            config.acceptance_min_cosine = acceptance_min_cosine;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CompactionPatch {
    dead_ratio: Option<f64>,
}

impl CompactionPatch {
    fn from_value(value: &Value, path: &str) -> Result<Self, CribleError> {
        let object = json::object(value, path)?;
        json::reject_unknown_keys(object, path, &["dead_ratio"])?;
        Ok(Self {
            dead_ratio: match object.get("dead_ratio") {
                Some(value) => Some(json::float(value, &format!("{path}.dead_ratio"))?),
                None => None,
            },
        })
    }

    fn apply(&self, config: &mut CompactionConfig) {
        if let Some(dead_ratio) = self.dead_ratio {
            config.dead_ratio = dead_ratio;
        }
    }
}

/// Applies a patch to `config`, or leaves it untouched on error.
///
/// Frozen settings are refused with [`ErrorCode::ImmutableSetting`] before any
/// mutation (INV-16); unknown keys are refused when the patch is parsed
/// (ADR-0031). A rejected patch changes nothing.
pub fn apply_patch(config: &mut IndexConfig, patch: &IndexPatch) -> Result<(), CribleError> {
    reject_immutable(config, patch)?;

    let mut candidate = config.clone();
    if let Some(name) = &patch.name {
        candidate.name.clone_from(name);
    }
    if let Some(model) = &patch.model {
        candidate.model.clone_from(model);
    }
    if let Some(reference_precision) = patch.reference_precision {
        candidate.reference_precision = reference_precision;
    }
    if let Some(cluster_by) = &patch.cluster_by {
        candidate.cluster_by.clone_from(cluster_by);
    }
    if let Some(lexical) = &patch.lexical {
        lexical.apply(&mut candidate.lexical);
    }
    if let Some(search) = &patch.search {
        search.apply(&mut candidate.search);
    }
    if let Some(ingest) = &patch.ingest {
        ingest.apply(&mut candidate.ingest);
    }
    if let Some(compaction) = &patch.compaction {
        compaction.apply(&mut candidate.compaction);
    }
    if let Some(schema) = &patch.schema {
        schema.apply(&mut candidate.schema)?;
    }
    candidate.schema.validate()?;
    candidate.ingest.validate("$.ingest")?;

    *config = candidate;
    Ok(())
}

fn reject_immutable(config: &IndexConfig, patch: &IndexPatch) -> Result<(), CribleError> {
    if let Some(name) = &patch.name
        && name != &config.name
    {
        return Err(immutable("name", "$.name"));
    }
    if let Some(model) = &patch.model
        && model != &config.model
    {
        return Err(immutable("model", "$.model"));
    }
    if let Some(reference_precision) = patch.reference_precision
        && reference_precision != config.reference_precision
    {
        return Err(immutable("reference_precision", "$.reference_precision"));
    }
    if let Some(schema) = &patch.schema {
        schema.reject_immutable(config)?;
    }
    Ok(())
}

fn immutable(setting: &str, path: &str) -> CribleError {
    CribleError::new(
        ErrorCode::ImmutableSetting,
        format!("setting '{setting}' is immutable at {path}; recreate the index to change it"),
    )
}

#[cfg(test)]
mod tests {
    use super::{IndexConfig, IngestConfig, Schema};
    use crate::error::ErrorCode;

    /// `to_json_string` round-trips through `from_json_str`, writes both
    /// options explicitly even when they took their default, and sorts the
    /// field names.
    #[test]
    fn schema_json_round_trips_with_explicit_options_and_sorted_keys() {
        let schema = Schema::from_json_str(
            r#"{
                "document_fields": {
                    "b": { "type": "keyword", "filterable": false },
                    "a": { "type": "integer" }
                },
                "passage_fields": {
                    "c": { "type": "bool", "returned": false }
                }
            }"#,
        )
        .expect("the test schema is valid");

        let text = schema.to_json_string();
        let reparsed = Schema::from_json_str(&text).expect("the serialized schema is valid");
        assert_eq!(schema, reparsed, "the schema must round-trip");

        let value: serde_json::Value = serde_json::from_str(&text).expect("the text is JSON");
        let document = value["document_fields"]
            .as_object()
            .expect("document_fields is an object");
        let a = document["a"].as_object().expect("a is an object");
        assert_eq!(a["filterable"], true, "the default is written explicitly");
        assert_eq!(a["returned"], true, "the default is written explicitly");
        let b = document["b"].as_object().expect("b is an object");
        assert_eq!(b["filterable"], false);
        assert_eq!(b["returned"], true);
        let passage = value["passage_fields"]
            .as_object()
            .expect("passage_fields is an object");
        assert_eq!(passage["c"]["returned"], false);

        assert!(
            text.find("\"a\"").expect("a is serialized")
                < text.find("\"b\"").expect("b is serialized"),
            "the document fields must be sorted: {text}"
        );
    }

    /// ADR-0054: `acceptance_min_cosine` is in `]0, 1]`, NaN included in the
    /// refused values. JSON cannot carry NaN, so the check is exercised on the
    /// structure directly.
    #[test]
    fn an_acceptance_threshold_of_nan_is_refused() {
        let ingest = IngestConfig {
            acceptance_sample: 1,
            acceptance_min_cosine: f64::NAN,
        };

        let error = ingest
            .validate("$.ingest")
            .expect_err("NaN is outside ]0, 1]");

        assert_eq!(error.code(), ErrorCode::InvalidRequest);
        assert!(
            error.message().contains("acceptance_min_cosine"),
            "the message names the setting: {}",
            error.message()
        );
    }

    /// Threshold the insertion tests use.
    const THRESHOLD: u32 = 1_234;

    /// A full, valid configuration whose `search` member is `search` and whose
    /// `name` is `name`, pretty-printed and without a trailing newline.
    fn pretty_configuration(search: &str, name: &str) -> String {
        let template = r#"{
  "name": "__NAME__",
  "model": "m",
  "reference_precision": "fp16",
  "schema": { "document_fields": {}, "passage_fields": {} },
  "cluster_by": ["juridiction"],
  "lexical": { "enabled": false, "language": "fr", "positions": false },
  "search": __SEARCH__,
  "ingest": { "acceptance_sample": 1, "acceptance_min_cosine": 0.9 },
  "compaction": { "dead_ratio": 0.1 }
}"#;
        template
            .replace("__NAME__", name)
            .replace("__SEARCH__", search)
    }

    /// The same configuration on one line, with no whitespace at all.
    fn compact_configuration(search: &str) -> String {
        let template = r#"{"name":"n","model":"m","reference_precision":"fp16","schema":{"document_fields":{},"passage_fields":{}},"cluster_by":["juridiction"],"lexical":{"enabled":false,"language":"fr","positions":false},"search":__SEARCH__,"ingest":{"acceptance_sample":1,"acceptance_min_cosine":0.9},"compaction":{"dead_ratio":0.1}}"#;
        template.replace("__SEARCH__", search)
    }

    /// `text` without the inserted `exact_threshold` member, and without the
    /// line the insertion added when it added one.
    fn without_threshold(text: &str) -> String {
        let start = text
            .find("\"exact_threshold\"")
            .unwrap_or_else(|| panic!("the stored form must name the threshold: {text}"));
        let colon = start + text[start..].find(':').expect("the member has a colon");
        let digits = colon
            + text[colon..]
                .find(|character: char| character.is_ascii_digit())
                .expect("the member has a number");
        let width = text[digits..]
            .chars()
            .take_while(char::is_ascii_digit)
            .count();
        let mut end = digits + width;
        while text[end..].starts_with(' ') {
            end += 1;
        }
        if text[end..].starts_with(',') {
            end += 1;
        }
        let line_start = text[..start].rfind('\n').map_or(start, |offset| offset + 1);
        let indentation_only = start > line_start
            && text[line_start..start]
                .chars()
                .all(|character| character == ' ' || character == '\t');
        if indentation_only {
            let after = text[end..]
                .find('\n')
                .map_or(end, |offset| end + offset + 1);
            format!("{}{}", &text[..line_start], &text[after..])
        } else {
            format!("{}{}", &text[..start], &text[end..])
        }
    }

    /// Materialises `raw`, checks the decoded result is `raw` plus the one key,
    /// and checks that every byte of `raw` survives outside the inserted member.
    fn assert_inserted(raw: &str) {
        let stored = IndexConfig::materialised_json(raw)
            .unwrap_or_else(|error| panic!("the configuration must be materialised: {error}"));
        let config = IndexConfig::from_json_str(&stored)
            .unwrap_or_else(|error| panic!("the stored configuration must parse: {error}"));
        assert_eq!(
            config.search.exact_threshold, THRESHOLD,
            "the inserted threshold must equal N"
        );

        let mut expected: serde_json::Value = serde_json::from_str(raw)
            .unwrap_or_else(|error| panic!("the fixture must be JSON: {error}"));
        expected["search"]["exact_threshold"] = serde_json::Value::from(THRESHOLD);
        let after: serde_json::Value = serde_json::from_str(&stored)
            .unwrap_or_else(|error| panic!("the stored configuration must be JSON: {error}"));
        assert_eq!(
            after, expected,
            "the insertion must add that one key and nothing else"
        );

        assert_eq!(
            without_threshold(&stored),
            raw,
            "every byte outside the inserted member must be unchanged"
        );
    }

    /// A string array before `search`, and strings holding `{`, `}` and escaped
    /// quotes: the scan must still find the top-level `search` member. A skipped
    /// `]` or `}` after a string is what the scan used to get wrong.
    #[test]
    fn materialised_json_finds_search_after_a_string_array_and_braces_in_strings() {
        let raw = pretty_configuration(
            r#"{ "scan": "binary", "rescore_candidates": 1234, "candidates_per_branch": 10, "fusion": { "method": "rrf" } }"#,
            r#"a { brace } and a \"quoted\" name"#,
        );
        assert!(!raw.ends_with('\n'), "the fixture has no trailing newline");
        assert_inserted(&raw);
    }

    /// An empty `search` object takes the member alone, with no leading comma;
    /// no valid configuration can reach it — `search.rescore_candidates` is
    /// required — so the helper is tested directly, compact and pretty.
    #[test]
    fn insert_search_threshold_completes_an_empty_object() {
        let compact = super::insert_search_threshold(r#"{"search":{}}"#, 7)
            .expect("an empty object is completable");
        assert_eq!(compact, r#"{"search":{"exact_threshold": 7}}"#);

        let pretty = super::insert_search_threshold("{\n  \"search\": {\n  }\n}", 7)
            .expect("an empty pretty object is completable");
        assert_eq!(
            pretty,
            "{\n  \"search\": {\n    \"exact_threshold\": 7\n  }\n}"
        );
    }

    /// A CRLF configuration keeps CRLF on the inserted line.
    #[test]
    fn materialised_json_keeps_crlf_line_endings() {
        let raw = pretty_configuration(
            "{\n    \"scan\": \"binary\",\n    \"rescore_candidates\": 1234,\n    \"candidates_per_branch\": 10,\n    \"fusion\": { \"method\": \"rrf\" }\n  }",
            "n",
        )
        .replace('\n', "\r\n");
        assert!(raw.contains("\"search\": {\r\n"), "the fixture is CRLF");
        let stored = IndexConfig::materialised_json(&raw)
            .unwrap_or_else(|error| panic!("the configuration must be materialised: {error}"));
        assert!(
            stored.contains("\r\n    \"exact_threshold\": 1234,"),
            "the inserted line must be introduced by CRLF: {stored}"
        );
        assert_inserted(&raw);
    }

    /// A compact configuration, with a string array and an object after a
    /// string value before `search`, keeps every byte.
    #[test]
    fn materialised_json_handles_a_compact_configuration() {
        let raw = compact_configuration(
            r#"{"scan":"binary","rescore_candidates":1234,"candidates_per_branch":10,"fusion":{"method":"rrf"}}"#,
        );
        assert!(!raw.contains('\n'), "the fixture is one line");
        assert_inserted(&raw);
    }
}
