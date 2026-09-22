//! Bulk export: turn a media query into a grant-gated bundle (M23-02).
//!
//! # The four things this module is careful about
//!
//! **A query walks through the reader's own filter surface.** `MediaQuery` and
//! `QueryAst` already enforce eligibility, so a cursor walk of the query can
//! leak no work the caller may not see — the filter is a parameter of the
//! query, not something applied after.
//!
//! **Every item is recorded before the bundle is built.** `bulk_export_items`
//! is the audit trail: a decision + reason per work, written as the walk
//! progresses. A bundle with no item rows is an error, not a success.
//!
//! **Bounds are refused at request time.** A `COUNT` preflight against the
//! same filters, with `max_items` (default 50) and `max_bytes` (default 1 GiB)
//! as config. Over the cap is a 422 naming the cap, not a ten-minute job that
//! ends in a shrug.
//!
//! **The output rides on the same download-grant machinery as a single work.**
//! The bundle is one `export_jobs` row (subject_type = 'query'), so the
//! `/grant` + `/download/{token}` routes work unchanged.

use lorehaven_domain::exports::ExportFormat;
use serde_json::{json, Value};

use crate::state::AppState;
use crate::worker::HandlerError;



/// Why a bulk-export worker stopped.
#[derive(Debug)]
pub enum BulkFailure {
    /// A database or storage failure: the attempt may be retried.
    Transient(String),
    /// A decision a retry cannot fix.
    Fatal(String),
    /// The query matched nothing eligible.
    Empty,
}

impl BulkFailure {
    /// The message recorded against the export.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Transient(message) | Self::Fatal(message) => message.clone(),
            Self::Empty => "no eligible works matched the query".to_owned(),
        }
    }

    /// Whether a retry might change the outcome.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(..))
    }
}

/// Carry out one queued bulk export: walk the query, render each eligible
/// work, bundle the artifacts, write the output, and record a download grant.
///
/// # Errors
/// Returns what the queue should act on.
pub async fn run_bulk(state: &AppState, payload: &Value) -> Result<(), HandlerError> {
    let export_job_id = payload
        .get(crate::exports::PAYLOAD_EXPORT_JOB_ID)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HandlerError::Fatal(format!(
                "a bulk export payload must carry {}",
                crate::exports::PAYLOAD_EXPORT_JOB_ID
            ))
        })?;

    let db = state.db();

    let row = lorehaven_db::exports::find_export(db, export_job_id)
        .await
        .map_err(|error| HandlerError::Transient(error.to_string()))?
        .ok_or_else(|| {
            HandlerError::Fatal(format!("bulk export {export_job_id} no longer exists"))
        })?;

    // Idempotent: a redelivered job for an export that is finished does nothing.
    if row.state == "ready" || row.state == "failed" {
        return Ok(());
    }

    lorehaven_db::exports::set_export_state(db, export_job_id, "running")
        .await
        .map_err(|error| HandlerError::Transient(error.to_string()))?;

    // The stored query is in options_json.
    let query_json: Value = row
        .options_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| json!({}));

    let account_id = &row.account_id;
    let max_items = state.config().bulk_export.max_items;
    let max_bytes = state.config().bulk_export.max_bytes;

    match produce_bulk(state, &row, &query_json, account_id, max_items, max_bytes).await {
        Ok(artifact) => {
            // Store the bundle blob and reference it.
            let store = lorehaven_db::storage::BlobStore::new(state.config().storage.root.clone());
            let (checksum, _key) = store
                .put(db, &artifact.bytes, &artifact.media_type)
                .await
                .map_err(|error| HandlerError::Transient(error.to_string()))?;

            store
                .reference(
                    db,
                    &checksum,
                    crate::exports::EXPORT_OWNER_TYPE,
                    export_job_id,
                )
                .await
                .map_err(|error| {
                    let _ = store.delete_if_unreferenced(db, &checksum);
                    HandlerError::Transient(error.to_string())
                })?;

            lorehaven_db::exports::record_output(
                db,
                export_job_id,
                &checksum,
                i64::try_from(artifact.bytes.len()).unwrap_or(i64::MAX),
                None,
            )
            .await
            .map_err(|error| HandlerError::Transient(error.to_string()))?;
            Ok(())
        }
        Err(failure) => {
            lorehaven_db::exports::fail_export(
                db,
                export_job_id,
                &json!({"code": "BULK_FAILED", "message": failure.message()}).to_string(),
            )
            .await
            .map_err(|error| HandlerError::Transient(error.to_string()))?;
            if failure.is_transient() {
                Err(HandlerError::Transient(failure.message()))
            } else {
                Err(HandlerError::Fatal(failure.message()))
            }
        }
    }
}

/// Walk the query, render eligible works, and bundle them into a ZIP.
///
/// Each visited work is recorded in `bulk_export_items` with its decision.
async fn produce_bulk(
    state: &AppState,
    export: &lorehaven_db::exports::ExportJob,
    query_json: &Value,
    account_id: &str,
    max_items: i64,
    max_bytes: i64,
) -> Result<Artifact, BulkFailure> {
    let db = state.db();
    let account_id_arg = if account_id.is_empty() {
        None
    } else {
        Some(account_id)
    };

    // Parse the stored query into a QueryAst, falling back to an empty query.
    // Parse the stored query into a QueryAst, extracting the "q" field from JSON.
    let query: Option<lorehaven_domain::query::QueryAst> = if query_json.is_null() {
        None
    } else {
        // Extract the "q" field (DSL string) from the JSON object
        let query_str = query_json.get("q").and_then(|v| v.as_str()).unwrap_or("");
        // Parse the DSL string, treating parse failure as fatal (never export everything)
        match lorehaven_domain::query::parse_query(query_str) {
            Ok(ast) => Some(ast),
            Err(e) => {
                // Invalid query in stored options is a data inconsistency - treat as fatal
                return Err(BulkFailure::Fatal(format!(
                    "stored query parse failed: {:?}",
                    e
                )));
            }
        }
    };

    // Preflight count: how many eligible works match?
    let (eligible_count, _) =
        lorehaven_db::media::count_media_filtered(db, query.as_ref(), account_id_arg)
            .await
            .map_err(|error| BulkFailure::Transient(error.to_string()))?;

    if eligible_count == 0 {
        return Err(BulkFailure::Empty);
    }

    if eligible_count > max_items {
        return Err(BulkFailure::Fatal(format!(
            "query matches {eligible_count} works, exceeding the cap of {max_items}. \
             Narrow the query or raise the cap."
        )));
    }

    // Walk the query with a cursor, rendering each work.
    let mut items: Vec<(String, Vec<u8>)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut total_bytes: i64 = 0;
    let limit = i64::min(eligible_count, max_items);
    let mut cursor: Option<String> = None;
    let export_id = &export.id;

    loop {
        let (page, _total, next_cursor) = lorehaven_db::media::list_media_filtered(
            db,
            query.as_ref(),
            account_id_arg,
            50,
            cursor.as_deref(),
            None,
            None,
            None,
            None,
        )
        .await
        .map_err(|error| BulkFailure::Transient(error.to_string()))?;

        if page.is_empty() {
            break;
        }

        for media in &page {
            let work_id = &media.id;
            // Render this work through the same path as a single export.
            let item_id = uuid::Uuid::new_v4().to_string();
            match render_work_to_bytes(state, work_id, account_id).await {
                Ok(bytes) => {
                    total_bytes += i64::try_from(bytes.len()).unwrap_or(i64::MAX);
                    if total_bytes > max_bytes {
                        // Record the skip and stop.
                        skipped.push((work_id.clone(), "over_bounds".to_string()));
                        lorehaven_db::exports::record_bulk_item(
                            db,
                            &item_id,
                            export_id,
                            work_id,
                            "skipped",
                            Some("over_bounds"),
                            None,
                            None,
                        )
                        .await
                        .map_err(|error| BulkFailure::Transient(error.to_string()))?;
                        return Err(BulkFailure::Fatal(format!(
                            "bundle would exceed the cap of {max_bytes} bytes"
                        )));
                    }
                    lorehaven_db::exports::record_bulk_item(
                        db,
                        &item_id,
                        export_id,
                        work_id,
                        "included",
                        None,
                        None,
                        Some(i64::try_from(bytes.len()).unwrap_or(i64::MAX)),
                    )
                    .await
                    .map_err(|error| BulkFailure::Transient(error.to_string()))?;
                    items.push((format!("{work_id}.html"), bytes));
                }
                Err(reason) => {
                    skipped.push((work_id.clone(), reason.clone()));
                    lorehaven_db::exports::record_bulk_item(
                        db,
                        &item_id,
                        export_id,
                        work_id,
                        "skipped",
                        Some(&reason),
                        None,
                        None,
                    )
                    .await
                    .map_err(|error| BulkFailure::Transient(error.to_string()))?;
                }
            }
        }

        cursor = next_cursor;
        if cursor.is_none() || items.len() as i64 >= limit {
            break;
        }
    }

    if items.is_empty() {
        return Err(BulkFailure::Empty);
    }

    // The manifest: every item with its decision, so the bundle is auditable
    // without opening the database. Included items carry their file name and
    // size; skipped ones carry the reason they were skipped.
    let manifest = build_manifest(export, &items, &skipped);
    let mut zip_items = items.clone();
    zip_items.push((
        "manifest.json".to_string(),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    ));

    // Build the ZIP bundle.
    let zip_bytes = build_zip(&zip_items).map_err(|error| BulkFailure::Transient(error))?;

    Ok(Artifact {
        bytes: zip_bytes,
        media_type: ExportFormat::Zip.media_type().to_owned(),
    })
}

/// Render a single work to HTML bytes through the same path as a single export.
async fn render_work_to_bytes(
    state: &AppState,
    work_id: &str,
    account_id: &str,
) -> Result<Vec<u8>, String> {
    let export_work = crate::exports::load_subject(state, account_id, "work", work_id)
        .await
        .map_err(|error| error.message().to_owned())?;

    lorehaven_domain::exports::render(
        &export_work,
        ExportFormat::Html,
        &lorehaven_domain::exports::ExportOptions::defaults(),
    )
    .map_err(|error| error.to_string())
}

/// A built artifact: bytes + media type.
struct Artifact {
    bytes: Vec<u8>,
    media_type: String,
}

/// The bundle manifest, embedded as `manifest.json` in every bulk export.
///
/// Mirrors the `bulk_export_items` audit table: one row per work the walk
/// visited, with the decision and, for skips, the reason. A reader can
/// verify what the bundle contains (and what it refused) without database
/// access.
fn build_manifest(
    export: &lorehaven_db::exports::ExportJob,
    included: &[(String, Vec<u8>)],
    skipped: &[(String, String)],
) -> Value {
    use serde_json::json;

    let included_json: Vec<Value> = included
        .iter()
        .map(|(name, bytes)| {
            json!({
                "work_id": name.trim_end_matches(".html"),
                "file": name,
                "bytes": bytes.len(),
                "decision": "included",
            })
        })
        .collect();
    let skipped_json: Vec<Value> = skipped
        .iter()
        .map(|(work_id, reason)| {
            json!({
                "work_id": work_id,
                "decision": "skipped",
                "reason": reason,
            })
        })
        .collect();

    json!({
        "export_id": export.id,
        "requested_at": export.created_at,
        "format": "zip",
        "included": included_json,
        "skipped": skipped_json,
        "totals": {
            "included": included.len(),
            "skipped": skipped.len(),
        }
    })
}

/// Build a ZIP archive from (name, bytes) pairs. Writes stored-method
/// (method 0) entries by hand — minimal but valid, every OS opens it.
fn build_zip(items: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    build_zip_stored(&mut buf, items);
    Ok(buf)
}

/// Build a stored-method ZIP by hand.
fn build_zip_stored(buf: &mut Vec<u8>, items: &[(String, Vec<u8>)]) {
    // Local file headers + data.
    let mut central_dir = Vec::new();
    let mut offset: u32 = 0;

    for (name, data) in items {
        let name_bytes = name.as_bytes();
        let crc = crc32fast::hash(data);

        // Local file header.
        buf.extend_from_slice(&0x04034b50u32.to_le_bytes()); // signature
        buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
        buf.extend_from_slice(&0u16.to_le_bytes()); // flags
        buf.extend_from_slice(&0u16.to_le_bytes()); // method (stored)
        buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
        buf.extend_from_slice(&0u16.to_le_bytes()); // mod date
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed size
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed size
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(data);

        // Central directory entry.
        central_dir.extend_from_slice(&0x02014b50u32.to_le_bytes()); // signature
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // flags
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // method
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central_dir.extend_from_slice(&crc.to_le_bytes());
        central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central_dir.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central_dir.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central_dir.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central_dir.extend_from_slice(&offset.to_le_bytes()); // local header offset
        central_dir.extend_from_slice(name_bytes);

        offset += 30 + name_bytes.len() as u32 + data.len() as u32;
    }

    let cd_start = buf.len() as u32;
    buf.extend_from_slice(&central_dir);
    let cd_end = buf.len() as u32;

    // End of central directory.
    buf.extend_from_slice(&0x06054b50u32.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // disk
    buf.extend_from_slice(&0u16.to_le_bytes()); // cd start disk
    buf.extend_from_slice(&(items.len() as u16).to_le_bytes()); // entries on disk
    buf.extend_from_slice(&(items.len() as u16).to_le_bytes()); // total entries
    buf.extend_from_slice(&(cd_end - cd_start).to_le_bytes()); // cd size
    buf.extend_from_slice(&cd_start.to_le_bytes()); // cd offset
    buf.extend_from_slice(&0u16.to_le_bytes()); // comment len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_zip_produces_valid_archive() {
        let items = vec![
            ("a.txt".to_owned(), b"hello".to_vec()),
            ("b.txt".to_owned(), b"world".to_vec()),
        ];
        let bytes = build_zip(&items).expect("build_zip");
        // A ZIP starts with the local file header signature.
        assert_eq!(&bytes[0..4], &[0x50, 0x4b, 0x03, 0x04]);
        // And ends with the end-of-central-directory record (22 bytes, no comment).
        assert!(bytes.len() >= 22, "ZIP too short: {}", bytes.len());
        assert_eq!(
            &bytes[bytes.len() - 22..bytes.len() - 18],
            &[0x50, 0x4b, 0x05, 0x06],
            "ZIP should end with EOCD record"
        );
    }
}
