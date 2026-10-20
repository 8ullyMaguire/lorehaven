//! The OpenAPI document and the route table must agree, in **both** directions.
//!
//! WHY THIS EXISTS
//!
//! The document used to be written out by hand. It drifted: it described four
//! paths, the instance served a hundred and fifty public ones, and the only
//! thing that noticed was a client generator producing a client that did not
//! work. **A document that is wrong in the direction of "less" is the one nobody
//! checks**, because a client that only calls what the document mentions works
//! perfectly — against an instance that has far more.
//!
//! So the document is now **generated from the declared route table**, and this
//! test asserts the two agree in both directions:
//!
//!   * **every public route is in the document** — a route that exists and is
//!     not documented is a door nobody knows about, and a client that cannot
//!     reach it;
//!   * **every path in the document resolves to a registered route** — a
//!     documented path that does not exist is a promise the instance does not
//!     keep, and a client that calls it gets a 404 it cannot explain.
//!
//! **The route table is the same declaration the route-inventory test uses**
//! (`crates/app/src/routes/inventory.rs`), so a route cannot be in one and not
//! the other.

use std::collections::BTreeSet;

use darkhold_app::routes::inventory::{Audience, ROUTE_TABLE};
use darkhold_app::routes::openapi;

fn document_paths() -> BTreeSet<String> {
    let document = openapi::document();
    document["paths"]
        .as_object()
        .expect("the document's paths are an object")
        .keys()
        .cloned()
        .collect()
}

fn table_paths() -> BTreeSet<String> {
    ROUTE_TABLE
        .iter()
        .filter(|entry| entry.audience == Audience::Public)
        // The table's paths are **relative**, because the document's `servers`
        // entry carries the `/api/v1` prefix and OpenAPI 3.1 resolves `paths`
        // against it. A path that carried the prefix itself would be
        // `/api/v1/api/v1/works` for a client that resolves it.
        .map(|entry| entry.path.to_string())
        .collect()
}

#[test]
fn every_public_route_is_in_the_document() {
    let documented = document_paths();
    let declared = table_paths();

    let missing: Vec<&String> = declared.difference(&documented).collect();
    assert!(
        missing.is_empty(),
        "{} public route(s) are registered and not in the OpenAPI document:\n  {}\n\
         A client that reads the document cannot reach them.",
        missing.len(),
        missing
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn every_documented_path_resolves_to_a_registered_route() {
    let documented = document_paths();
    let declared = table_paths();

    let extra: Vec<&String> = documented.difference(&declared).collect();
    assert!(
        extra.is_empty(),
        "{} path(s) in the OpenAPI document are not registered routes:\n  {}\n\
         A client that calls them gets a 404 it cannot explain.",
        extra.len(),
        extra
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn the_document_is_openapi_31_and_says_so() {
    let document = openapi::document();
    assert_eq!(document["openapi"], "3.1.0");
    assert_eq!(document["info"]["title"], "Darkhold Public API");
    assert!(
        document["components"]["schemas"]["Work"].is_object(),
        "the schemas are still there: a document with no schemas is a document that \
         describes nothing"
    );
}

#[test]
fn the_documented_paths_are_relative_to_the_server() {
    // OpenAPI 3.1 resolves `paths` against the `servers` URL, and the document's
    // `servers` entry says the base is `/api/v1`. **A path that carried the
    // prefix itself would be `/api/v1/api/v1/works`** for a client that resolves
    // it — which is the kind of drift a generated document is supposed to make
    // impossible, and which an existing test in `milestone_45.rs` caught.
    let document = openapi::document();
    assert_eq!(
        document["servers"][0]["url"], "/api/v1",
        "the server entry carries the prefix, so the paths must not"
    );
    for path in document_paths() {
        assert!(
            !path.starts_with("/api/v1"),
            "{path} carries the prefix the servers entry already carries"
        );
    }
}

#[test]
fn a_path_with_both_a_public_and_an_authenticated_variant_is_published() {
    // A path can be reachable two ways: `GET /works/{id}` may be public while
    // `DELETE /works/{id}` needs a session. **The path is public**, so it is in
    // the document — and the document says nothing about which method is which,
    // which is why the document is a *surface* description and not a contract
    // for every method.
    //
    // This test exists because the naive reading — "a path with a
    // `RequireSession` entry must not be published" — is wrong, and it caught
    // this implementation's first attempt.
    let documented = document_paths();
    let public_paths: BTreeSet<&str> = ROUTE_TABLE
        .iter()
        .filter(|entry| entry.audience == Audience::Public)
        .map(|entry| entry.path)
        .collect();
    for entry in ROUTE_TABLE {
        let path = entry.path.to_string();
        if public_paths.contains(entry.path) {
            assert!(
                documented.contains(&path),
                "{path} has a public variant and must be in the document"
            );
        }
    }
}

#[test]
fn the_document_is_served_at_the_path_the_table_names() {
    // The door and the document are one thing, and the table names both.
    let served = ROUTE_TABLE
        .iter()
        .find(|entry| entry.handler == "get_openapi_spec")
        .expect("the table names the document's handler");
    assert_eq!(served.path, "/openapi.json");
    assert_eq!(served.method, "GET");
    assert_eq!(served.audience, Audience::Public);
}
