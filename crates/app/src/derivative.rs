//! Derivative pipeline worker logic (spec §32.4, M25).
//!
//! Produces EPUB/PDF/text renditions of source blobs using the instance's
//! Calibre/Pandoc converters, mirrors the export worker pattern.

use anyhow::{bail, Context};

use crate::state::AppState;
use crate::worker::HandlerError;
use lorehaven_domain::JobId;

pub const DERIVATIVE_OWNER_TYPE: &str = "derivative";

/// Handle a Derivative job: convert a parent blob into a rendition.
pub async fn handle_derivative(
    state: &AppState,
    _job: JobId,
    derivative_id: &str,
) -> Result<(), HandlerError> {
    let db = state.db();
    let derivative = lorehaven_db::derivative::find_derivative(db, derivative_id)
        .await
        .map_err(|e| HandlerError::Transient(e.to_string()))?
        .ok_or_else(|| {
            HandlerError::Fatal(format!("derivative {derivative_id} not found"))
        })?;

    let kind = lorehaven_domain::derivative::DerivativeKind::parse(&derivative.derivative_kind)
        .ok_or_else(|| {
            HandlerError::Fatal(format!("unknown derivative_kind: {}", derivative.derivative_kind))
        })?;

    let store = lorehaven_db::storage::BlobStore::new(state.config().storage.root.clone());
    let parent_bytes = store
        .get(db, &derivative.parent_checksum)
        .await
        .map_err(|e| HandlerError::Transient(e.to_string()))?
        .ok_or_else(|| {
            HandlerError::Fatal(format!(
                "parent blob {} not found for derivative {}",
                derivative.parent_checksum, derivative_id
            ))
        })?;

    let output = produce_rendition(state, kind, &parent_bytes, &derivative.edition_kind)
        .await
        .map_err(|e| HandlerError::Transient(e.to_string()))?;

    let (output_checksum, _key) = store
        .put(db, &output.bytes, &output.media_type)
        .await
        .map_err(|e| HandlerError::Transient(e.to_string()))?;

    store
        .reference(db, &output_checksum, DERIVATIVE_OWNER_TYPE, derivative_id)
        .await
        .map_err(|e| HandlerError::Transient(e.to_string()))?;

    lorehaven_db::derivative::mark_derivative_built(
        db,
        derivative_id,
        &output_checksum,
        output.bytes.len() as i64,
        &output.media_type,
    )
    .await
    .map_err(|e| HandlerError::Transient(e.to_string()))?;

    Ok(())
}

/// The output of a rendition.
struct Rendition {
    bytes: Vec<u8>,
    media_type: String,
}

/// Produce a rendition based on the derivative kind.
async fn produce_rendition(
    state: &AppState,
    kind: lorehaven_domain::derivative::DerivativeKind,
    source: &[u8],
    _edition_kind: &str,
) -> anyhow::Result<Rendition> {
    match kind {
        lorehaven_domain::derivative::DerivativeKind::Epub => {
            convert_with(state, "epub", source, "application/epub+zip").await
        }
        lorehaven_domain::derivative::DerivativeKind::Pdf => {
            convert_with(state, "pdf", source, "application/pdf").await
        }
        lorehaven_domain::derivative::DerivativeKind::Text => {
            convert_with(state, "txt", source, "text/plain; charset=utf-8").await
        }
        // OCR and transcode require specialized binaries (Tesseract, ffmpeg);
        // refuse at enqueue time rather than fail here.
        other => bail!("{other} not yet supported by this instance"),
    }
}

/// Convert a source blob using Calibre's `ebook-convert` or pandoc.
async fn convert_with(
    state: &AppState,
    target_format: &str,
    source: &[u8],
    media_type: &str,
) -> anyhow::Result<Rendition> {
    let converters = state.converters();
    let ebook_convert = converters
        .program(lorehaven_domain::exports::Converter::EbookConvert)
        .ok_or_else(|| anyhow::anyhow!("ebook-convert not installed"))?;

    // Write source to a temp file, convert, read output.
    let temp_dir = std::env::temp_dir().join(format!("lorehaven-derivative-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir).await?;
    let source_path = temp_dir.join("source.html");
    tokio::fs::write(&source_path, source).await?;
    let output_path = temp_dir.join(format!("output.{target_format}"));

    let output = tokio::process::Command::new(ebook_convert)
        .arg(&source_path)
        .arg(&output_path)
        .output()
        .await
        .with_context(|| format!("running ebook-convert for {target_format}"))?;

    if !output.status.success() {
        bail!(
            "ebook-convert failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let bytes = tokio::fs::read(&output_path)
        .await
        .with_context(|| format!("reading output {target_format}"))?;

    // Clean up temp dir.
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    Ok(Rendition {
        bytes,
        media_type: media_type.to_string(),
    })
}
