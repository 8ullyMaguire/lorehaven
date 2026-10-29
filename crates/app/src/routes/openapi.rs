//! The OpenAPI document.
//!
//! **It is generated from the declared route table, not hand-written.**
//!
//! A hand-written document drifts: a route is added, the document is not updated,
//! and the two disagree in the only way that matters — the document says a thing
//! exists that does not, or does not say a thing exists that does. A client
//! generator reads the document and produces a client that does not work, and
//! nothing fails.
//!
//! So the paths come from [`ROUTE_TABLE`], which is the **same declaration** the
//! route-inventory test uses. A route cannot be published without being declared,
//! and a declared route cannot be published without being documented — and the
//! OpenAPI test asserts both directions (spec §20.4).
//!
//! **The public surface only.** A door that needs a session, a pseud, a token
//! or the operator is not part of the public contract, and publishing it would
//! be a promise the instance does not make to an anonymous reader.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use crate::auth::MaybeSession;
use crate::state::AppState;

use super::inventory::{Audience, ROUTE_TABLE};

/// The prefix every route in the table is mounted under.
const API_PREFIX: &str = "/api/v1";

/// The OpenAPI routes.
pub fn router() -> Router<AppState> {
    Router::new().route("/openapi.json", get(get_openapi_spec))
}

/// Serve the document.
pub async fn get_openapi_spec(_maybe: MaybeSession, State(_state): State<AppState>) -> Json<Value> {
    Json(document())
}

/// Build the OpenAPI 3.1 document.
///
/// **The paths are generated from the route table.** Everything else — the
/// version, the info block, the security schemes and the schemas — is fixed,
/// because those are prose and a schema, and a schema that is generated from a
/// route table would be a schema of nothing.
#[must_use]
pub fn document() -> Value {
    let mut paths: Map<String, Value> = Map::new();

    for entry in ROUTE_TABLE {
        // The public surface, and only the public surface.
        if entry.audience != Audience::Public {
            continue;
        }
        // The paths are **relative to the `servers` entry**, which is where
        // `/api/v1` lives. OpenAPI 3.1's `paths` are resolved against the server
        // URL, so a path that carried the prefix itself would be
        // `/api/v1/api/v1/works` for a client that resolves it. The table's
        // paths are already relative — the table is mounted under `/api/v1` —
        // and the document says so once, in `servers`.
        let path = entry.path.to_string();
        let method = entry.method.to_ascii_lowercase();
        let operation = json!({
            "operationId": format!("{}_{}", method, entry.handler),
            "summary": format!("{} {}", entry.method, entry.path),
            // WHO MAY CALL THIS, in the document itself.
            //
            // The route table has carried an audience per entry for as long as it has existed, and
            // `every_route_has_correct_audience` asserts it — but that test reads the TABLE. The
            // document built from the table left the field out, so the one artifact a third party
            // reads said nothing about who may call what. Measured against the deployed instance:
            // 158 operations, 0 carrying `x-audience`, while gravity's document annotated 353/353.
            //
            // Emitting it is what makes the published contract an answer to "what may I call",
            // rather than a list of paths and a hope.
            "x-audience": entry.audience.label(),
            "tags": [tag_for(entry.path)],
            "responses": {
                "200": { "description": "the response" },
                "404": { "description": "no such thing, or not for you" },
            },
        });
        let item = paths.entry(path).or_insert_with(|| json!({}));
        if let Some(object) = item.as_object_mut() {
            object.insert(method, operation);
        }
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Darkhold Public API",
            "version": "v1",
            "description": "Supported public interface for Darkhold. Administrative and \
                           experimental endpoints are marked separately and require \
                           additional scopes.",
            "contact": { "name": "API Support", "url": "/docs" },
        },
        "servers": [{ "url": API_PREFIX, "description": "Current instance" }],
        "components": {
            "securitySchemes": {
                "bearer": { "type": "http", "scheme": "bearer", "bearerFormat": "UUID" },
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "properties": {
                        "error": { "type": "string" },
                        "field": { "type": "string", "nullable": true },
                        "code": { "type": "string", "nullable": true },
                    },
                },
                "Work": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "summary": { "type": "string" },
                        "language": { "type": "string" },
                        "rating": { "type": "string" },
                        "lifecycle": { "type": "string" },
                        "completion": { "type": "string" },
                        "published_at": { "type": "string", "format": "date-time", "nullable": true },
                    },
                },
                "Token": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "scopes": { "type": "array", "items": { "type": "string" } },
                        "created_at": { "type": "string", "format": "date-time" },
                    },
                },
            },
        },
        "paths": paths,
    })
}

/// The tag an operation is filed under: the first segment of its path.
///
/// A client generator groups operations by tag, and a document with one tag per
/// operation is a document no generator can group.
fn tag_for(path: &str) -> String {
    let first = path
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("root");
    if first.starts_with('{') {
        "root".to_string()
    } else {
        first.to_string()
    }
}
