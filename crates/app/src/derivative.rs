//! Derivative pipeline worker logic (spec §32.4, M25).
//!
//! Produces EPUB/PDF/text renditions of source blobs using the instance's
//! Calibre/Pandoc converters, OCR text with Tesseract, and a streaming MP4 for
//! uploaded media with ffmpeg. Which program a kind needs is
//! `DerivativeKind::required_programs`, so the request door can refuse with the
//! same remedy this module would otherwise fail with.

use anyhow::{bail, Context};

use crate::state::AppState;
use crate::worker::HandlerError;
use lorehaven_domain::derivative::DerivativeKind;
use lorehaven_domain::JobId;

pub const DERIVATIVE_OWNER_TYPE: &str = "derivative";

/// How a derivative build failed.
///
/// The distinction is what the job system does next: a missing parent blob or
/// an unknown kind cannot be fixed by trying again, and retrying a job whose
/// answer is deterministic burns attempts and fills the log. A conversion that
/// died half way through can be retried.
enum BuildFailure {
    Fatal(String),
    Transient(String),
}

impl BuildFailure {
    fn fatal(message: impl Into<String>) -> Self {
        Self::Fatal(message.into())
    }

    fn transient(error: impl std::fmt::Display) -> Self {
        Self::Transient(error.to_string())
    }

    fn message(&self) -> &str {
        match self {
            Self::Fatal(message) | Self::Transient(message) => message,
        }
    }
}

/// Handle a Derivative job: convert a parent blob into a rendition.
pub async fn handle_derivative(
    state: &AppState,
    _job: JobId,
    derivative_id: &str,
) -> Result<(), HandlerError> {
    match build_rendition(state, derivative_id).await {
        Ok(()) => Ok(()),
        Err(failure) => {
            // The row is what an operator and the verification sweep read; a
            // failure recorded only on the job row would leave the derivative
            // looking queued forever.
            let message = failure.message().to_owned();
            if let Err(error) = lorehaven_db::derivative::mark_derivative_failed(
                state.db(),
                derivative_id,
                &message,
            )
            .await
            {
                tracing::warn!(
                    derivative = derivative_id,
                    error = %error,
                    "could not record the derivative failure on its row"
                );
            }
            match failure {
                BuildFailure::Fatal(message) => Err(HandlerError::Fatal(message)),
                BuildFailure::Transient(message) => Err(HandlerError::Transient(message)),
            }
        }
    }
}

/// Load what the job names, produce the rendition and store it.
async fn build_rendition(state: &AppState, derivative_id: &str) -> Result<(), BuildFailure> {
    let db = state.db();
    let derivative = lorehaven_db::derivative::find_derivative(db, derivative_id)
        .await
        .map_err(BuildFailure::transient)?
        .ok_or_else(|| BuildFailure::fatal(format!("derivative {derivative_id} not found")))?;

    let kind = DerivativeKind::parse(&derivative.derivative_kind).ok_or_else(|| {
        BuildFailure::fatal(format!(
            "unknown derivative_kind: {}",
            derivative.derivative_kind
        ))
    })?;

    let store = lorehaven_db::storage::BlobStore::new(state.config().storage.root.clone());
    let parent_bytes = store
        .get(db, &derivative.parent_checksum)
        .await
        .map_err(BuildFailure::transient)?
        .ok_or_else(|| {
            BuildFailure::fatal(format!(
                "parent blob {} not found for derivative {}",
                derivative.parent_checksum, derivative_id
            ))
        })?;
    // The parent's recorded content type is what tells Tesseract and ffmpeg how
    // to read the temporary file; the checksum alone does not.
    let parent_type = store
        .stat(db, &derivative.parent_checksum)
        .await
        .map_err(BuildFailure::transient)?
        .map(|stat| stat.content_type)
        .unwrap_or_else(|| "application/octet-stream".to_owned());

    // A program this instance does not have is fatal: retrying cannot install
    // it, and the message names what the operator has to do.
    let output = produce_rendition(state, kind, &parent_bytes, &parent_type)
        .await
        .map_err(|error| {
            if kind.required_programs().is_empty() {
                BuildFailure::transient(error)
            } else {
                // A machine-produced kind's only causes of failure are a
                // missing program or an input the program refused; both are
                // reported by the program itself and neither improves with a
                // retry on its own.
                BuildFailure::fatal(error.to_string())
            }
        })?;

    let (output_checksum, _key) = store
        .put(db, &output.bytes, &output.media_type)
        .await
        .map_err(BuildFailure::transient)?;

    store
        .reference(db, &output_checksum, DERIVATIVE_OWNER_TYPE, derivative_id)
        .await
        .map_err(BuildFailure::transient)?;

    lorehaven_db::derivative::mark_derivative_built(
        db,
        derivative_id,
        &output_checksum,
        output.bytes.len() as i64,
        &output.media_type,
    )
    .await
    .map_err(BuildFailure::transient)?;

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
    kind: DerivativeKind,
    source: &[u8],
    source_type: &str,
) -> anyhow::Result<Rendition> {
    match kind {
        DerivativeKind::Epub => convert_with(state, "epub", source).await,
        DerivativeKind::Pdf => convert_with(state, "pdf", source).await,
        DerivativeKind::Text => convert_with(state, "txt", source).await,
        DerivativeKind::Ocr => recognise(state, source, source_type).await,
        DerivativeKind::Transcode => transcode(state, source, source_type).await,
    }
}

/// A scratch directory that removes itself when it goes out of scope.
///
/// Every path in this module hands an external program a real file, and a
/// failure part-way through is exactly when a `let _ = remove_dir_all(...)` at
/// the end of the happy path does not run. Drop order is the guarantee.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    async fn new(tag: &str) -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(format!("lorehaven-{tag}-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self(dir))
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The filename extension a program should see for a recorded content type.
///
/// Neither Tesseract nor ffmpeg sniffs reliably enough to be trusted with a
/// file called `source.bin`; they mostly key off the extension and only some
/// formats are probed by content. An unknown type gets `.bin`, which is honest
/// — the program then says it cannot read the input, and that message reaches
/// the job's error instead of a wrong guess at a format.
fn extension_for(content_type: &str) -> &'static str {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/tiff" => "tiff",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/gif" => "gif",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "application/pdf" => "pdf",
        "text/html" => "html",
        "text/plain" => "txt",
        _ => "bin",
    }
}

/// Run a program with fixed argv and capture its stdout as bytes.
///
/// No shell: the arguments are passed as a vector, so a checksum or a filename
/// can never become a command. A non-zero exit carries the program's own
/// stderr, which is what an operator needs to read.
async fn run_capture(program: &std::path::Path, args: &[&str]) -> anyhow::Result<Vec<u8>> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .with_context(|| format!("running {}", program.display()))?;
    if !output.status.success() {
        bail!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// Find a required program, or say which one is missing.
fn required_program(state: &AppState, kind: DerivativeKind) -> anyhow::Result<std::path::PathBuf> {
    let converters = state.converters();
    for program in kind.required_programs() {
        if let Some(path) = converters.discover_program(program) {
            return Ok(path);
        }
    }
    bail!(
        "{} is required to produce a {kind} derivative and is not installed: {}",
        kind.required_programs().join(" or "),
        kind.install_hint()
    )
}

/// OCR a scan with Tesseract, returning the recognised text.
///
/// `tesseract <input> stdout` writes the text to stdout, which avoids a second
/// temp file and a second chance to leak one.
async fn recognise(
    state: &AppState,
    source: &[u8],
    source_type: &str,
) -> anyhow::Result<Rendition> {
    let kind = DerivativeKind::Ocr;
    let tesseract = required_program(state, kind)?;

    let scratch = ScratchDir::new("ocr").await?;
    let input = scratch
        .path()
        .join(format!("source.{}", extension_for(source_type)));
    tokio::fs::write(&input, source).await?;

    let input_arg = input.to_string_lossy().to_string();
    let text = run_capture(&tesseract, &[&input_arg, "stdout"]).await?;

    Ok(Rendition {
        bytes: text,
        media_type: kind.output_media_type().to_string(),
    })
}

/// Normalize uploaded media into a streaming-friendly MP4 with ffmpeg.
///
/// The container and the `+faststart` flag are the decision: a media work is
/// served progressively by a browser, and an MP4 whose index sits at the end of
/// the file plays only after the whole thing has downloaded. Re-encoding
/// (`libx264`/`aac`) rather than stream-copying is deliberate — a stream copy
/// would preserve whatever codec the upload happened to carry, which is what
/// transcoding exists to normalize.
async fn transcode(
    state: &AppState,
    source: &[u8],
    source_type: &str,
) -> anyhow::Result<Rendition> {
    let kind = DerivativeKind::Transcode;
    let ffmpeg = required_program(state, kind)?;

    let scratch = ScratchDir::new("transcode").await?;
    let input = scratch
        .path()
        .join(format!("source.{}", extension_for(source_type)));
    tokio::fs::write(&input, source).await?;
    let output = scratch.path().join("output.mp4");

    let input_arg = input.to_string_lossy().to_string();
    let output_arg = output.to_string_lossy().to_string();
    run_capture(
        &ffmpeg,
        &[
            "-nostdin",
            "-y",
            "-i",
            &input_arg,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
            &output_arg,
        ],
    )
    .await?;

    let bytes = tokio::fs::read(&output)
        .await
        .with_context(|| format!("reading the transcoded output at {}", output.display()))?;

    Ok(Rendition {
        bytes,
        media_type: kind.output_media_type().to_string(),
    })
}

/// Convert a source blob using Calibre's `ebook-convert` or pandoc.
async fn convert_with(
    state: &AppState,
    target_format: &str,
    source: &[u8],
) -> anyhow::Result<Rendition> {
    let converters = state.converters();
    let ebook_convert = converters
        .program(lorehaven_domain::exports::Converter::EbookConvert)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ebook-convert is required to produce a {target_format} derivative and is not \
                 installed: install Calibre (ebook-convert) or pandoc"
            )
        })?;

    // Write source to a temp file, convert, read output.
    let scratch = ScratchDir::new("derivative").await?;
    let source_path = scratch.path().join("source.html");
    tokio::fs::write(&source_path, source).await?;
    let output_path = scratch.path().join(format!("output.{target_format}"));

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

    Ok(Rendition {
        bytes,
        media_type: media_type_for_format(target_format).to_string(),
    })
}

/// The media type an export format's artifact carries.
fn media_type_for_format(target_format: &str) -> &'static str {
    match target_format {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        _ => "text/plain; charset=utf-8",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_follow_the_recorded_content_type() {
        assert_eq!(extension_for("image/png"), "png");
        assert_eq!(extension_for("image/jpeg; charset=binary"), "jpg");
        assert_eq!(extension_for("video/quicktime"), "mov");
        assert_eq!(extension_for("  IMAGE/PNG  "), "png");
        // An unknown type is honest rather than a guess: the program then says
        // it cannot read the file, and that message reaches the job's error.
        assert_eq!(extension_for("application/octet-stream"), "bin");
    }

    #[test]
    fn a_scratch_directory_removes_itself() {
        let path = {
            let scratch = tokio::runtime::Runtime::new()
                .expect("runtime")
                .block_on(ScratchDir::new("test"))
                .expect("scratch");
            let path = scratch.path().to_path_buf();
            assert!(path.exists(), "the directory must exist while held");
            path
        };
        assert!(
            !path.exists(),
            "the directory must be gone once the guard is dropped"
        );
    }
}
