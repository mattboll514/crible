// SPDX-License-Identifier: Apache-2.0

// Tests may panic; `allow-panic-in-tests` covers `#[test]` functions only, and
// the helpers of this file are assertions.
#![allow(clippy::expect_used, clippy::panic)]

//! The route list of `docs/05-api.md` and the paths of the generated
//! specification are the same set, in both directions.
//!
//! References: `docs/05-api.md` (the table of the endpoints and the paragraph
//! that makes it the reference), ADR-0015 (the specification is generated from
//! the code) and ADR-0111, decisions 5 and 9 (the comparison, the two
//! publication routes and the routes a later phase delivers).

use std::collections::BTreeSet;

/// The reference document, read at compile time.
const DOCS: &str = include_str!("../../../docs/05-api.md");

/// The methods an operation may carry in the specification.
const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// The two publication routes: static resources, outside the specification.
///
/// ADR-0111, decision 5, excepts them by name in both directions.
const PUBLICATION: [(&str, &str); 2] = [("GET", "/openapi.json"), ("GET", "/docs")];

/// The rows of the table a later task delivers, excepted by name until then.
///
/// ADR-0111, decision 9: the exception is written here with the task that
/// delivers the route, and the change that mounts the route removes it.
const LATER: [(&str, &str); 2] = [
    // ROUTE-STATS, P7 (`docs/phases/P7-observabilite.md`).
    ("GET", "/v1/stats"),
    // JOURNAL-DE-DEBUG, P7 (`docs/phases/P7-observabilite.md`).
    ("GET", "/v1/debug/queries"),
];

/// One couple (method, path without query string) of the table.
fn documented_routes() -> BTreeSet<(String, String)> {
    let mut routes = BTreeSet::new();
    let mut in_table = false;
    for line in DOCS.lines() {
        if line.starts_with("| Méthode et chemin |") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if !line.starts_with('|') {
            break;
        }
        let cells: Vec<&str> = line.split('|').collect();
        assert!(
            cells.len() >= 4,
            "the row of the table has a method cell, a role and a description: {line}"
        );
        // The separator row of the table holds dashes, not a route.
        if !cells[1].contains('`') {
            continue;
        }
        // A cell may hold two routes, separated by `·`; a path may carry a
        // query string, which never appears in the specification.
        for item in cells[1].split('·') {
            let item = item.trim().trim_matches('`').trim();
            if item.is_empty() || item == "Méthode et chemin" {
                continue;
            }
            let (method, path) = item
                .split_once(' ')
                .unwrap_or_else(|| panic!("the cell `{item}` holds a method and a path"));
            let path = path.split('?').next().unwrap_or(path);
            routes.insert((method.trim().to_owned(), path.to_owned()));
        }
    }
    assert!(!routes.is_empty(), "the table of the endpoints is found");
    routes
}

/// The couples the specification describes.
fn specified_routes() -> BTreeSet<(String, String)> {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    let paths = specification["paths"]
        .as_object()
        .expect("the specification has a paths object");
    let mut routes = BTreeSet::new();
    for (path, item) in paths {
        let item = item
            .as_object()
            .unwrap_or_else(|| panic!("the path item of `{path}` is an object"));
        for method in item.keys() {
            if METHODS.contains(&method.as_str()) {
                routes.insert((method.to_uppercase(), path.clone()));
            }
        }
    }
    routes
}

fn excepted() -> BTreeSet<(String, String)> {
    PUBLICATION
        .iter()
        .chain(LATER.iter())
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect()
}

#[test]
fn the_specification_and_the_table_agree_in_both_directions() {
    let documented = documented_routes();
    let specified = specified_routes();
    let excepted = excepted();

    // An exception names a row that exists: a route delivered by a later phase
    // stays in the table, and the publication routes stay documented.
    for (method, path) in excepted.iter() {
        assert!(
            documented.contains(&(method.clone(), path.clone())),
            "the exception `{method} {path}` names a row of the table of `docs/05-api.md`"
        );
    }

    let missing_from_the_specification: Vec<_> = documented
        .difference(&specified)
        .filter(|route| !excepted.contains(*route))
        .collect();
    assert!(
        missing_from_the_specification.is_empty(),
        "every documented route is in the specification: {missing_from_the_specification:?}"
    );

    let missing_from_the_table: Vec<_> = specified
        .difference(&documented)
        .filter(|route| !excepted.contains(*route))
        .collect();
    assert!(
        missing_from_the_table.is_empty(),
        "every route of the specification is documented in `docs/05-api.md`: \
         {missing_from_the_table:?}"
    );
}

#[test]
fn the_specification_documents_the_role_of_every_operation() {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    for (path, item) in specification["paths"]
        .as_object()
        .expect("the specification has a paths object")
    {
        for (method, operation) in item
            .as_object()
            .unwrap_or_else(|| panic!("the path item of `{path}` is an object"))
        {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            let description = operation["description"]
                .as_str()
                .unwrap_or_else(|| panic!("`{method} {path}` has a description"));
            assert!(
                description.contains("Role:"),
                "the description of `{method} {path}` names the required role: {description}"
            );
        }
    }
}

/// The body bound is a layer of the whole instance, so every operation may
/// answer `413` (ADR-0102, decision 1).
#[test]
fn every_operation_declares_the_body_bound() {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    let mut operations = 0;
    for (path, item) in specification["paths"]
        .as_object()
        .expect("the specification has a paths object")
    {
        for (method, operation) in item
            .as_object()
            .unwrap_or_else(|| panic!("the path item of `{path}` is an object"))
        {
            if !METHODS.contains(&method.as_str()) {
                continue;
            }
            operations += 1;
            let response = &operation["responses"]["413"];
            assert!(
                response.is_object(),
                "`{method} {path}` declares `413`, the body bound (ADR-0102)"
            );
            assert_eq!(
                response["content"]["application/json"]["schema"]["$ref"],
                "#/components/schemas/Error",
                "`{method} {path}` answers `413` with the error body"
            );
        }
    }
    assert_eq!(
        operations, 18,
        "every operation of `docs/05-api.md` is covered"
    );
}

#[test]
fn the_two_probes_need_no_key_in_the_specification() {
    let specification = crible_server::openapi::specification()
        .expect("the specification of the server is serialisable");
    for path in ["/health", "/ready"] {
        let operation = &specification["paths"][path]["get"];
        let requirements = operation["security"].as_array().unwrap_or_else(|| {
            panic!(
                "`GET {path}` declares its own security requirement, which cancels the global one"
            )
        });
        // `utoipa` renders `security(())` as `[{}]`: one requirement that names
        // no scheme.
        assert!(
            !requirements.is_empty(),
            "`GET {path}` cancels the global requirement"
        );
        for requirement in requirements {
            let schemes = requirement
                .as_object()
                .unwrap_or_else(|| panic!("a security requirement of `GET {path}` is an object"));
            assert!(
                schemes.is_empty(),
                "`GET {path}` requires no scheme: {schemes:?}"
            );
        }
    }
    // A route with a role inherits the global requirement and declares none.
    assert!(
        specification["paths"]["/v1/indexes"]["post"]
            .get("security")
            .is_none(),
        "a role route inherits the global requirement"
    );
    let global = specification["security"]
        .as_array()
        .expect("the document requires `bearerAuth`");
    assert_eq!(global.len(), 1, "one global requirement: {global:?}");
    assert!(
        global[0].get("bearerAuth").is_some(),
        "the global requirement names `bearerAuth`: {global:?}"
    );
}
