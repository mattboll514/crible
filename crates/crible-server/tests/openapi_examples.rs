// SPDX-License-Identifier: Apache-2.0

// Tests may panic; `allow-panic-in-tests` covers `#[test]` functions only, and
// the helpers of this file are assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! Every JSON example of `docs/05-api.md` is valid JSON and conforms to the
//! component of the generated specification it illustrates.
//!
//! References: `docs/05-api.md` (the examples of the bodies), ADR-0111,
//! decision 8 (the marker `<!-- openapi: … -->`, the `none` case and the
//! validation against the generated schema) and ADR-0015 (the specification is
//! generated from the code).

use serde_json::Value;

/// The reference document, read at compile time.
const DOCS: &str = include_str!("../../../docs/05-api.md");

/// One fenced `json` block of the document.
struct Example {
    /// Line of the opening fence, for the messages.
    line: usize,
    /// Component of the specification, or `None` when the marker is `none`.
    component: Option<String>,
    /// The parsed block.
    value: Value,
}

/// Reads the marker written just above the opening fence.
///
/// The marker is `<!-- openapi: NomDuComposant -->`, or `<!-- openapi: none —
/// raison -->` when no component describes the block (ADR-0111, decision 8).
fn marker(line: &str) -> Option<String> {
    let inner = line.trim().strip_prefix("<!-- openapi:")?;
    let inner = inner.strip_suffix("-->")?;
    Some(inner.trim().to_owned())
}

/// Parses every fenced `json` block and its marker.
fn examples() -> Vec<Example> {
    let lines: Vec<&str> = DOCS.lines().collect();
    let mut examples = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "```json" {
            index += 1;
            continue;
        }
        let opening = index;
        let mut above = opening;
        while above > 0 && lines[above - 1].trim().is_empty() {
            above -= 1;
        }
        assert!(
            above > 0,
            "the block of line {} is preceded by a marker",
            opening + 1
        );
        let annotation = marker(lines[above - 1]).unwrap_or_else(|| {
            panic!(
                "the JSON block of line {} needs `<!-- openapi: Composant -->` \
                 or `<!-- openapi: none — raison -->` (ADR-0111, decision 8)",
                opening + 1
            )
        });

        let mut body = String::new();
        index += 1;
        while index < lines.len() && lines[index].trim() != "```" {
            body.push_str(lines[index]);
            body.push('\n');
            index += 1;
        }
        assert!(
            index < lines.len(),
            "the JSON block of line {} is closed",
            opening + 1
        );
        let value: Value = serde_json::from_str(&body).unwrap_or_else(|error| {
            panic!(
                "the JSON block of line {} is valid JSON: {error}\n{body}",
                opening + 1
            )
        });
        // The marker is a component name, or `none` followed by a dash and a
        // reason: a name that merely starts with `none` is a component name.
        let component = if annotation == "none" {
            None
        } else if let Some(reason) = annotation.strip_prefix("none \u{2014}") {
            assert!(
                !reason.trim().is_empty(),
                "`none` carries its reason in the marker of line {} (ADR-0111, decision 8)",
                opening + 1
            );
            None
        } else {
            Some(annotation)
        };
        examples.push(Example {
            line: opening + 1,
            component,
            value,
        });
        index += 1;
    }
    assert!(!examples.is_empty(), "the document holds JSON examples");
    examples
}

/// The root schema of one component: the whole specification, with a `$ref` on
/// the component. `$ref` has no remote target, so the validation reads nothing.
fn validator(component: &str) -> jsonschema::Validator {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    assert!(
        specification["components"]["schemas"]
            .get(component)
            .is_some(),
        "the specification declares the component `{component}`"
    );
    let mut root = specification;
    root["$ref"] = Value::String(format!("#/components/schemas/{component}"));
    jsonschema::validator_for(&root).unwrap_or_else(|error| {
        panic!("the schema of `{component}` compiles: {error}");
    })
}

#[test]
fn json_examples_conform_to_the_specification() {
    for example in examples() {
        let Some(component) = example.component else {
            continue;
        };
        let validator = validator(&component);
        if let Err(error) = validator.validate(&example.value) {
            panic!(
                "the example of line {} conforms to `{component}`: {error}\n{}",
                example.line, example.value
            );
        }
    }
}

/// The harness itself rejects a wrong body in both directions: an object where
/// an array is expected, and an unknown key where the server refuses one.
#[test]
fn the_validation_rejects_a_wrong_body() {
    let list = validator("IndexList");
    assert!(
        list.validate(&serde_json::json!({ "indexes": "not a list" }))
            .is_err(),
        "a wrong type is refused"
    );
    let deposited = validator("DepositedBatchRequest");
    assert!(
        deposited
            .validate(&serde_json::json!({ "directory": "x", "extra": 1 }))
            .is_err(),
        "an unknown key is refused by a closed body"
    );
    assert!(
        deposited
            .validate(&serde_json::json!({ "directory": "x" }))
            .is_ok(),
        "the documented body is accepted"
    );
}

/// The schema now refuses the bodies the server refuses, and keeps accepting
/// the ones it accepts (ADR-0111, decision 7).
#[test]
fn the_schema_refuses_what_the_server_refuses() {
    let fusion = validator("Fusion");
    assert!(
        fusion
            .validate(&serde_json::json!({ "method": "rrf" }))
            .is_ok(),
        "`rrf` alone is accepted"
    );
    assert!(
        fusion
            .validate(&serde_json::json!({ "method": "relative_score", "alpha": 0.5 }))
            .is_ok(),
        "`relative_score` with its weight is accepted"
    );
    assert!(
        fusion
            .validate(&serde_json::json!({ "method": "rrf", "alpha": 0.5 }))
            .is_err(),
        "`alpha` with `rrf` is refused (ADR-0090)"
    );
    assert!(
        fusion
            .validate(&serde_json::json!({ "method": "relative_score", "alpha": 5 }))
            .is_err(),
        "`alpha` above 1 is refused (ADR-0090)"
    );
    assert!(
        fusion
            .validate(&serde_json::json!({ "method": "relative_score" }))
            .is_err(),
        "`relative_score` requires its weight"
    );

    let mode = validator("BatchMode");
    assert!(
        mode.validate(&serde_json::json!("upsert")).is_ok(),
        "`upsert` is accepted"
    );
    assert!(
        mode.validate(&serde_json::json!("replace_all")).is_err(),
        "`replace_all` is refused by the online route"
    );

    let score = validator("ScoreType");
    for value in ["cosine", "bm25", "rrf", "relative"] {
        assert!(
            score.validate(&serde_json::json!(value)).is_ok(),
            "`{value}` is a score type of `docs/03-recherche.md`"
        );
    }
    let multi = validator("MultiScoreType");
    assert!(
        multi.validate(&serde_json::json!("rrf")).is_ok(),
        "`rrf` is a multi-index score"
    );
    assert!(
        multi.validate(&serde_json::json!("bm25")).is_err(),
        "a multi-index score is `cosine` or `rrf` (ADR-0110)"
    );

    let document = validator("OnlineDocument");
    assert!(
        document
            .validate(&serde_json::json!({
                "doc_id": "D1",
                "passages": [ { "text": "x" } ],
            }))
            .is_ok(),
        "the `fields` of a document and of a passage are optional"
    );
    assert!(
        document
            .validate(&serde_json::json!({
                "doc_id": "D1",
                "fields": { "juridiction": "cass" },
                "passages": [ { "text": "x", "fields": { "partie": "motifs" } } ],
            }))
            .is_ok(),
        "the `fields` may be given"
    );
}

#[test]
fn every_marker_is_a_valid_component_or_none() {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    for example in examples() {
        let Some(component) = example.component else {
            continue;
        };
        assert!(
            specification["components"]["schemas"]
                .get(&component)
                .is_some(),
            "the marker of line {} names a component of the specification: `{component}`",
            example.line
        );
    }
}
