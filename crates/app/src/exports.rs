//! Exports: the service behind `/exports` and the worker's `export` handler.
//!
//! # Three things this file is careful about
//!
//! **The work is read through the reader's own path.** An export is a copy of
//! what the reader may read, so it loads the same document, the same chapters and
//! the same imported bodies as the reader's pages. A second door into the content
//! is how an export quietly becomes a way to read something the reader could not
//! open.
//!
//! **A converter is discovered, never assumed.** `ebook-convert` and `pandoc` are
//! looked for on `PATH` at startup and their versions recorded. A format whose
//! converter is missing is *offered as unavailable with installation guidance*
//! rather than queued and failed, because "your PDF failed" and "this instance has
//! no PDF converter" are different sentences to a reader (spec §13.1).
//!
//! **Arguments are fixed and nothing goes through a shell.** The only variable
//! part of a converter invocation is the path this instance itself wrote, in its
//! own storage, with a name this instance chose.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lorehaven_db::exports as repo;
use lorehaven_db::exports::{ExportJob, NewExport};
use lorehaven_db::storage::BlobStore;
use lorehaven_domain::exports::{
    render, Converter, ExportChapter, ExportError, ExportFormat, ExportOptions, ExportProvenance,
    ExportWork,
};
use lorehaven_domain::jobs::{JobKind, RetryPolicy};
use lorehaven_domain::MediaAssetId;
use serde_json::{json, Value};

use crate::state::AppState;
use crate::worker::HandlerError;
use lorehaven_db::exports::cta_exemption;
use lorehaven_domain::exports::{CtaPlacementSerde, InstanceCta};

/// A repository fault: the attempt may be retried.
///
/// Local to this module for the same reason the importer has its own pair: what
/// counts as transient is a judgement about *this* work, and a shared helper
/// would invite one module's judgement onto another's failures.
fn transient(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::Transient(error.to_string())
}

/// A decision a retry cannot change.
fn fatal(message: impl Into<String>) -> HandlerError {
    HandlerError::Fatal(message.into())
}

/// The payload key a queued export carries.
pub const PAYLOAD_EXPORT_JOB_ID: &str = "export_job_id";

/// How an export's output blob is owned, so the collector leaves it alone.
pub const EXPORT_OWNER_TYPE: &str = "export_job";



///
/// The limit exists so that a pathological input — a work whose HTML sends a
/// converter into a long loop — cannot occupy a worker indefinitely. It is
/// generous because the cost of being wrong is a failed export.
pub const CONVERT_TIMEOUT: Duration = Duration::from_secs(120);

/// The largest artifact this instance will store from a converter.
///
/// A converter that emits more than this is not producing an export of the work
/// it was given.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Converters
// ---------------------------------------------------------------------------

/// The external converters this instance found, if any.
///
/// Detection is at startup and the result is carried in state, so an instance
/// whose Calibre was installed later says so until it restarts rather than
/// changing what it offers between two requests.
#[derive(Debug, Clone, Default)]
pub struct Converters {
    ebook_convert: Option<PathBuf>,
    pandoc: Option<PathBuf>,
}

impl Converters {
    /// Look for the converters on `PATH`.
    #[must_use]
    pub fn detect() -> Self {
        Self {
            ebook_convert: which("ebook-convert"),
            pandoc: which("pandoc"),
        }
    }

    /// The program that would serve a format, if this instance has one.
    #[must_use]
    pub fn program(&self, converter: Converter) -> Option<&Path> {
        match converter {
            Converter::EbookConvert => self.ebook_convert.as_deref(),
            Converter::Pandoc => self.pandoc.as_deref(),
        }
    }

    /// A format's converter and its program, in the order the format prefers.
    #[must_use]
    pub fn for_format(&self, format: ExportFormat) -> Option<(Converter, &Path)> {
        format
            .converters()
            .iter()
            .find_map(|converter| self.program(*converter).map(|path| (*converter, path)))
    }

    /// Whether this instance can produce a format.
    #[must_use]
    pub fn can_produce(&self, format: ExportFormat) -> bool {
        format.is_builtin() || self.for_format(format).is_some()
    }

    /// Find any program on `PATH` by name.
    ///
    /// The converters this struct holds are a fixed pair; this is for the other
    /// programs the same startup discovery answers for (Piper for narration,
    /// Tesseract and ffmpeg for derivatives). One `which` implementation, so a
    /// binary found for one purpose is found by the same rule for every purpose.
    #[must_use]
    pub fn discover_program(&self, program: &str) -> Option<PathBuf> {
        which(program)
    }

    /// What the API reports, including what to install for what is missing.
    #[must_use]
    pub fn report(&self) -> Vec<Value> {
        ExportFormat::ALL
            .iter()
            .map(|format| {
                let available = self.can_produce(*format);
                let converter = self.for_format(*format).map(|(converter, _)| converter);
                json!({
                    "format": format.as_str(),
                    "label": format.label(),
                    "media_type": format.media_type(),
                    "extension": format.extension(),
                    "builtin": format.is_builtin(),
                    "available": available,
                    // A reader who wanted a PDF learns what the operator would
                    // have to install, instead of a bare "not available".
                    "requires": if available { Value::Null } else { json!(format.requires()) },
                    "converter": converter.map(lorehaven_domain::exports::Converter::binary),
                    "converter_version": converter.and_then(|c| self.version(c)),
                })
            })
            .collect()
    }

    /// A converter's version, asked once per call — `--version` is cheap and it
    /// is only asked when a catalogue is built or an export is recorded.
    #[must_use]
    pub fn version(&self, converter: Converter) -> Option<String> {
        let program = self.program(converter)?;
        let output = std::process::Command::new(program)
            .arg("--version")
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let first = text.lines().next()?.trim();
        if first.is_empty() {
            None
        } else {
            Some(first.to_owned())
        }
    }
}

/// Find a program on `PATH`.
///
/// Split from `doctor` rather than shared with it: the doctor reports what is
/// missing to an operator, this decides what to offer a reader, and the two want
/// different answers when the answer is "no".
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

// ---------------------------------------------------------------------------
// The artifact
// ---------------------------------------------------------------------------

/// What an export produced.
#[derive(Debug, Clone)]
pub struct Artifact {
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// The media type to serve it as.
    pub media_type: String,
    /// The extension to name it with.
    pub extension: String,
    /// The converter that produced it, when one did.
    pub converter: Option<Converter>,
    /// The converter's own reported version, recorded as evidence.
    pub converter_version: Option<String>,
}

/// Why an export could not be produced.
#[derive(Debug)]
pub enum ExportFailure {
    /// The reader asked for something this instance cannot do. Fatal: retrying
    /// changes nothing.
    Unsupported(String),
    /// The work is not there, or has nothing to export.
    Empty(String),
    /// Reading or writing failed. Transient: it may work next time.
    Storage(String),
}

impl ExportFailure {
    /// The message a reader is shown.
    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Self::Unsupported(message) | Self::Empty(message) | Self::Storage(message) => message,
        }
    }

    /// Whether a retry could change the outcome.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Storage(_))
    }

    /// A stable code, so a client can branch without reading prose.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported(_) => "unsupported_format",
            Self::Empty(_) => "nothing_to_export",
            Self::Storage(_) => "storage_error",
        }
    }
}

// ---------------------------------------------------------------------------
// Loading a subject
// ---------------------------------------------------------------------------

/// The subject kinds an export may be asked for.
#[must_use]
/// The subject types this instance exports.
pub fn is_known_subject(subject_type: &str) -> bool {
    matches!(subject_type, "work" | "library_item" | "query")
}

/// Load a work or a library item into the shape the renderer wants.
///
/// # Errors
/// Returns [`ExportFailure::Empty`] when there is nothing to export, which is a
/// refusal rather than a bug: an export of no chapters is a file that lies about
/// what the reader asked for.
pub async fn load_subject(
    state: &AppState,
    account_id: &str,
    subject_type: &str,
    subject_id: &str,
) -> Result<ExportWork, ExportFailure> {
    match subject_type {
        "work" => load_work(state, subject_id).await,
        // A library item is private to the account that imported it, so this
        // path is scoped where the work path is not: a work is readable by
        // whoever may read it, a reader's library is readable by the reader.
        "library_item" => load_library_item(state, account_id, subject_id).await,
        other => Err(ExportFailure::Unsupported(format!(
            "{other:?} is not something this instance exports"
        ))),
    }
}

async fn load_work(state: &AppState, work_id: &str) -> Result<ExportWork, ExportFailure> {
    let db = state.db();
    let work_id = work_id
        .parse::<lorehaven_domain::WorkId>()
        .map_err(|_| ExportFailure::Empty("that work does not exist".to_owned()))?;
    let work = lorehaven_db::content::find_work(db, work_id)
        .await
        .map_err(|error| ExportFailure::Storage(error.to_string()))?
        .ok_or_else(|| ExportFailure::Empty("that work does not exist".to_owned()))?;

    let contributors = lorehaven_db::collaboration::contributors_for_work(db, work.id)
        .await
        .map_err(|error| ExportFailure::Storage(error.to_string()))?;

    // The credited names, publicly attributed ones only: an export carries an
    // attribution line, and a private co-author is a private co-author here too.
    let mut names = Vec::new();
    for contributor in contributors.iter().filter(|c| c.public_attribution) {
        if let Some(pseud) = lorehaven_db::identity::find_pseud(db, contributor.pseud_id)
            .await
            .map_err(|error| ExportFailure::Storage(error.to_string()))?
        {
            names.push(pseud.display_name);
        }
    }
    if names.is_empty() {
        if let Some(pseud) = lorehaven_db::identity::find_pseud(db, work.owner_pseud_id)
            .await
            .map_err(|error| ExportFailure::Storage(error.to_string()))?
        {
            names.push(pseud.display_name);
        }
    }

    let chapters = lorehaven_db::content::chapters_for_work(db, work.id)
        .await
        .map_err(|error| ExportFailure::Storage(error.to_string()))?;

    let mut exported = Vec::new();
    for (index, chapter) in chapters.iter().enumerate() {
        let Some(revision_id) = chapter.current_revision_id else {
            continue;
        };
        let document = lorehaven_db::content::revision_document(db, revision_id)
            .await
            .map_err(|error| ExportFailure::Storage(error.to_string()))?;
        let Some(document) = document else {
            continue;
        };
        let document = lorehaven_domain::document::Document::from_json(&document)
            .map_err(|error| ExportFailure::Storage(error.to_string()))?;
        exported.push(ExportChapter::authored(
            u32::try_from(index).unwrap_or(0) + 1,
            chapter.title.clone(),
            document,
        ));
    }

    if exported.is_empty() {
        return Err(ExportFailure::Empty(
            "that work has no chapters to export yet".to_owned(),
        ));
    }

    Ok(ExportWork {
        title: work.title.clone(),
        author: names.join(", "),
        language: work.language.clone(),
        chapters: exported,
        provenance: None,
        identifier: lorehaven_domain::exports::stable_identifier(&[
            "work",
            &work.id.to_string(),
            work.updated_at.as_str(),
        ]),
        modified: work.updated_at.clone(),
    })
}

async fn load_library_item(
    state: &AppState,
    account_id: &str,
    item_id: &str,
) -> Result<ExportWork, ExportFailure> {
    let db = state.db();
    let item = lorehaven_db::imports::get_library_item(db, item_id, account_id)
        .await
        .map_err(|error| ExportFailure::Storage(error.to_string()))?
        .ok_or_else(|| ExportFailure::Empty("that library item does not exist".to_owned()))?;

    let chapters = lorehaven_db::imports::latest_chapters_for_item(db, item_id)
        .await
        .map_err(|error| ExportFailure::Storage(error.to_string()))?;

    let store = BlobStore::new(state.config().storage.root.clone());
    let mut exported = Vec::new();
    for chapter in chapters.iter().filter(|c| c.state == "stored") {
        let Some(checksum) = chapter.content_blob_checksum.clone() else {
            continue;
        };
        let Some(bytes) = store
            .get(db, &checksum)
            .await
            .map_err(|error| ExportFailure::Storage(error.to_string()))?
        else {
            continue;
        };
        let html = String::from_utf8_lossy(&bytes).into_owned();
        exported.push(ExportChapter::imported(
            u32::try_from(chapter.ordinal).unwrap_or(0),
            chapter.title.clone(),
            html,
        ));
    }

    if exported.is_empty() {
        return Err(ExportFailure::Empty(
            "that import has no stored chapters to export".to_owned(),
        ));
    }

    let synced = item
        .last_synced_at
        .clone()
        .or_else(|| item.source_updated_at.clone())
        .unwrap_or_else(lorehaven_db::identity::now_rfc3339);

    Ok(ExportWork {
        title: item.title.clone(),
        author: item.author_text.clone(),
        language: item.language.clone().unwrap_or_else(|| "und".to_owned()),
        chapters: exported,
        // An imported work's provenance is the point of recording it: the file
        // says where it came from and when it was read.
        provenance: Some(ExportProvenance {
            source_name: item.source_key.clone(),
            source_url: item.source_url.clone(),
            retrieved_at: synced.clone(),
            source_key: Some(item.source_work_key.clone()),
            permission: None,
        }),
        identifier: lorehaven_domain::exports::stable_identifier(&[
            "library_item",
            item_id,
            synced.as_str(),
        ]),
        modified: synced,
    })
}

// ---------------------------------------------------------------------------
// Producing one
// ---------------------------------------------------------------------------

/// Produce an export's artifact.
///
/// # Errors
/// Returns an [`ExportFailure`] whose message is meant for a reader.
pub async fn produce(state: &AppState, row: &ExportJob) -> Result<Artifact, ExportFailure> {
    let format = ExportFormat::parse(&row.format).ok_or_else(|| {
        ExportFailure::Unsupported(format!("{:?} is not an export format", row.format))
    })?;
    let mut options = ExportOptions::from_json(row.options_json.as_deref());
    let work = load_subject(state, &row.account_id, &row.subject_type, &row.subject_id).await?;
    apply_instance_cta(state, &row.subject_id, &mut options).await?;

    if format.is_builtin() {
        let bytes = render(&work, format, &options).map_err(|error| match error {
            ExportError::Empty => {
                ExportFailure::Empty("there is nothing to put in that export".to_owned())
            }
            ExportError::NeedsConverter { format } => ExportFailure::Unsupported(format!(
                "{} needs a converter this instance does not have",
                format.label()
            )),
            ExportError::Epub(error) => {
                ExportFailure::Storage(format!("the EPUB could not be assembled: {error}"))
            }
            ExportError::ZipOnlyForBulk => ExportFailure::Unsupported(
                "ZIP format is only available for bulk exports".to_owned(),
            ),
        })?;
        return Ok(Artifact {
            bytes,
            media_type: format.media_type().to_owned(),
            extension: format.extension().to_owned(),
            converter: None,
            converter_version: None,
        });
    }

    let (converter, program) = state.converters().for_format(format).ok_or_else(|| {
        ExportFailure::Unsupported(format!(
            "{} needs {} installed on this instance",
            format.label(),
            format.requires()
        ))
    })?;

    // The converter's input is this instance's own standalone HTML: it is the
    // rendering already verified for the HTML format, and using it means a PDF
    // and an HTML export of the same work cannot disagree.
    let html = render(&work, ExportFormat::Html, &options).map_err(|error| match error {
        ExportError::Empty => {
            ExportFailure::Empty("there is nothing to put in that export".to_owned())
        }
        ExportError::NeedsConverter { .. } => {
            ExportFailure::Unsupported("HTML is rendered by this instance".to_owned())
        }
        ExportError::Epub(error) => {
            ExportFailure::Storage(format!("the intermediate could not be built: {error}"))
        }
        ExportError::ZipOnlyForBulk => {
            ExportFailure::Unsupported("ZIP format is only available for bulk exports".to_owned())
        }
    })?;

    let bytes = convert(state, program, converter, format, &html).await?;
    Ok(Artifact {
        bytes,
        media_type: format.media_type().to_owned(),
        extension: format.extension().to_owned(),
        converter: Some(converter),
        converter_version: state.converters().version(converter),
    })
}

/// Run a converter over this instance's own HTML.
///
/// The temporary directory is inside the instance's storage root, so the
/// converter writes where this instance already has permission and cleanup is a
/// single directory that is removed whatever happens.
async fn convert(
    state: &AppState,
    program: &Path,
    converter: Converter,
    format: ExportFormat,
    html: &[u8],
) -> Result<Vec<u8>, ExportFailure> {
    let dir = state
        .config()
        .storage
        .root
        .join("tmp")
        .join(format!("export-{}", MediaAssetId::new()));
    tokio::fs::create_dir_all(&dir).await.map_err(|error| {
        ExportFailure::Storage(format!("could not prepare a work directory: {error}"))
    })?;

    let input = dir.join("input.html");
    let output = dir.join(format!("output.{}", format.extension()));
    tokio::fs::write(&input, html)
        .await
        .map_err(|error| ExportFailure::Storage(format!("could not write the input: {error}")))?;

    // Fixed arguments, one path this instance chose, no shell. `ebook-convert`
    // and `pandoc` both take input-then-output here, and `pandoc` is told the
    // formats rather than left to guess from extensions.
    let mut command = tokio::process::Command::new(program);
    match converter {
        Converter::EbookConvert => {
            command.arg("--quiet").arg(&input).arg(&output);
        }
        Converter::Pandoc => {
            command
                .arg("--from")
                .arg("html")
                .arg("--to")
                .arg(match format {
                    ExportFormat::Pdf => "pdf",
                    _ => "html",
                })
                .arg("--output")
                .arg(&output)
                .arg(&input);
        }
    }
    command.kill_on_drop(true);

    let result = tokio::time::timeout(CONVERT_TIMEOUT, command.output()).await;
    let outcome = match result {
        Err(_) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(ExportFailure::Storage(format!(
                "{} took longer than {} seconds",
                converter.binary(),
                CONVERT_TIMEOUT.as_secs()
            )));
        }
        Ok(Err(error)) => {
            let _ = tokio::fs::remove_dir_all(&dir).await;
            return Err(ExportFailure::Unsupported(format!(
                "{} could not be run: {error}",
                converter.binary()
            )));
        }
        Ok(Ok(output)) => output,
    };

    if !outcome.status.success() {
        let stderr = String::from_utf8_lossy(&outcome.stderr);
        let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join("; ");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(ExportFailure::Unsupported(format!(
            "{} exited with {}: {tail}",
            converter.binary(),
            outcome.status
        )));
    }

    let metadata = tokio::fs::metadata(&output).await.map_err(|error| {
        ExportFailure::Storage(format!("{} produced no file: {error}", converter.binary()))
    })?;
    if metadata.len() > MAX_ARTIFACT_BYTES {
        let _ = tokio::fs::remove_dir_all(&dir).await;
        return Err(ExportFailure::Unsupported(format!(
            "{} produced {} bytes, past this instance's limit",
            converter.binary(),
            metadata.len()
        )));
    }

    let bytes = tokio::fs::read(&output)
        .await
        .map_err(|error| ExportFailure::Storage(format!("could not read the output: {error}")))?;
    let _ = tokio::fs::remove_dir_all(&dir).await;
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// The worker's half
// ---------------------------------------------------------------------------

/// Carry out one queued export.
///
/// # Errors
/// Returns what the queue should act on: transient for a failure that might
/// clear, fatal for what a retry cannot change.
pub async fn run(state: &AppState, payload: &Value) -> Result<(), HandlerError> {
    let export_job_id = payload
        .get(PAYLOAD_EXPORT_JOB_ID)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            fatal(format!(
                "an export payload must carry {PAYLOAD_EXPORT_JOB_ID}"
            ))
        })?;

    let db = state.db();
    let row = repo::find_export(db, export_job_id)
        .await
        .map_err(transient)?
        .ok_or_else(|| fatal(format!("export {export_job_id} no longer exists")))?;

    // Idempotent: a redelivered job for an export that is finished does nothing.
    // The queue is at-least-once, so this is the difference between a retry and
    // a second artifact charged to the reader's quota.
    if row.state == "ready" || row.state == "failed" {
        return Ok(());
    }

    repo::set_export_state(db, export_job_id, "running")
        .await
        .map_err(transient)?;

    let artifact = match produce(state, &row).await {
        Ok(artifact) => artifact,
        Err(failure) => {
            repo::fail_export(
                db,
                export_job_id,
                &json!({ "code": failure.code(), "message": failure.message() }).to_string(),
            )
            .await
            .map_err(transient)?;
            return if failure.is_transient() {
                Err(transient(failure.message().to_owned()))
            } else {
                Err(fatal(failure.message().to_owned()))
            };
        }
    };

    // The output is referenced, not merely stored: an unreferenced blob is one
    // the collector is entitled to delete, and an export whose file vanished is
    // worse than one that failed.
    let store = BlobStore::new(state.config().storage.root.clone());
    let (checksum, _key) = store
        .put(db, &artifact.bytes, &artifact.media_type)
        .await
        .map_err(|error| transient(error.to_string()))?;
    if let Err(error) = store
        .reference(db, &checksum, EXPORT_OWNER_TYPE, export_job_id)
        .await
    {
        let _ = store.delete_if_unreferenced(db, &checksum).await;
        return Err(transient(error.to_string()));
    }

    repo::record_output(
        db,
        export_job_id,
        &checksum,
        i64::try_from(artifact.bytes.len()).unwrap_or(i64::MAX),
        artifact.converter_version.as_deref(),
    )
    .await
    .map_err(transient)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Asking for one
// ---------------------------------------------------------------------------

/// Create an export job and queue it.
///
/// # Errors
/// Refuses a format this instance cannot produce *before* a job exists, and
/// refuses a subject that would export to nothing — the same shape of refusal the
/// importer uses for a source it cannot read.
pub async fn request(
    state: &AppState,
    account_id: &str,
    subject_type: &str,
    subject_id: &str,
    format: ExportFormat,
    options: &ExportOptions,
) -> Result<ExportJob, HandlerError> {
    if !is_known_subject(subject_type) {
        return Err(fatal(format!(
            "{subject_type:?} is not something this instance exports"
        )));
    }
    if !state.converters().can_produce(format) {
        return Err(fatal(format!(
            "{} needs {} installed on this instance",
            format.label(),
            format.requires()
        )));
    }

    let db = state.db();
    // A stored file, which is what an export's output is.
    let export_id = MediaAssetId::new().to_string();

    // The subject is loaded once here, before the job exists, so that a work with
    // no chapters is refused in the request rather than in a worker the reader
    // never sees.
    load_subject(state, account_id, subject_type, subject_id)
        .await
        .map_err(|failure| fatal(failure.message().to_owned()))?;

    // Order matters and is not free to change, for the same reason it matters in
    // the importer. The queue row is written *first*, because the export row
    // holds a foreign key to it: inserting the export first fails outright under
    // enforced foreign keys. The export's id is chosen here rather than by the
    // insert, so the payload can name it from the moment the row is visible — a
    // job queued first and patched afterwards leaves a worker able to claim a job
    // pointing at an export that is not there.
    let job_id = lorehaven_db::jobs::enqueue(
        db,
        JobKind::Export,
        // The payload names the export and nothing else: an export of a private
        // work must not put the work's identity, or the reader's, in a queue row.
        &json!({ PAYLOAD_EXPORT_JOB_ID: export_id }).to_string(),
        None,
        Some(account_id.parse().unwrap_or_default()),
        0,
        &RetryPolicy::default(),
    )
    .await
    .map_err(transient)?;

    let row = repo::create_export(
        db,
        NewExport {
            id: &export_id,
            job_id: &job_id.to_string(),
            account_id,
            subject_type,
            subject_id,
            format: format.as_str(),
            options_json: Some(&options.to_json().to_string()),
        },
    )
    .await
    .map_err(transient)?;

    Ok(row)
}

/// Request a bulk export: a query turned into a grant-gated bundle.
///
/// The query is stored on the export row (`subject_type = 'query'`,
/// `options_json` holds the query + caps). A preflight count checks the
/// configured `max_items` cap; over the cap is a 422 naming the cap. The
/// job walks the query in the worker via [`crate::bulk_export::run_bulk`].
///
/// # Errors
/// Transient for a storage failure; a validation error if the query is
/// unknown or over the cap.
pub async fn request_bulk(
    state: &AppState,
    account_id: &str,
    query_json: &Value,
) -> Result<ExportJob, HandlerError> {
    let db = state.db();

    // Preflight count: how many eligible works match the query?
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
                return Err(HandlerError::Fatal(format!(
                    "stored query parse failed: {:?}",
                    e
                )));
            }
        }
    };
    let account_id_arg = if account_id.is_empty() {
        None
    } else {
        Some(account_id)
    };
    let (eligible_count, _) =
        lorehaven_db::media::count_media_filtered(db, query.as_ref(), account_id_arg)
            .await
            .map_err(transient)?;

    if eligible_count == 0 {
        return Err(fatal("no eligible works matched the query".to_owned()));
    }

    let max_items = state.config().bulk_export.max_items;
    if eligible_count > max_items {
        return Err(fatal(format!(
            "query matches {eligible_count} works, exceeding the cap of {max_items}. \
             Narrow the query or raise the cap."
        )));
    }

    let export_id = MediaAssetId::new().to_string();
    let job_id = lorehaven_db::jobs::enqueue(
        db,
        JobKind::BulkExport,
        &json!({ PAYLOAD_EXPORT_JOB_ID: export_id }).to_string(),
        None,
        Some(account_id.parse().unwrap_or_default()),
        0,
        &RetryPolicy::default(),
    )
    .await
    .map_err(transient)?;

    let row = repo::create_export(
        db,
        NewExport {
            id: &export_id,
            job_id: &job_id.to_string(),
            account_id,
            subject_type: "query",
            subject_id: "", // query is in options_json, not a single subject
            format: "zip",
            options_json: Some(&query_json.to_string()),
        },
    )
    .await
    .map_err(transient)?;

    Ok(row)
}

/// Mint a download grant, returning the token a URL carries.
///
/// Only the hash is stored, so a database read does not yield a usable URL.
///
/// # Errors
/// Transient for a storage failure.
pub async fn mint_grant(state: &AppState, export_job_id: &str) -> Result<String, HandlerError> {
    let token = crate::crypto::generate_token();
    let hash = crate::crypto::hash_token(&token);
    let ttl = state.config().exports.grant_ttl_secs;
    let expires_at = lorehaven_db::identity::in_seconds(ttl);
    repo::mint_grant(
        state.db(),
        &MediaAssetId::new().to_string(),
        export_job_id,
        &hash,
        &expires_at,
    )
    .await
    .map_err(transient)?;
    Ok(token)
}

/// Carry out one queued bulk export.
///
/// # Errors
/// Returns what the queue should act on: transient for a failure that might
/// clear, fatal for what a retry cannot change.
pub async fn run_bulk(_state: &AppState, _payload: &Value) -> Result<(), HandlerError> {
    Err(fatal("bulk export handler not yet implemented"))
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// Delete exports past their retention window, with their output.
///
/// The blob is unreferenced rather than deleted: the same output may be something
/// else's snapshot — content is shared by checksum across the whole instance — and
/// an export must not be able to delete a reader's reading copy.
pub async fn sweep(state: &AppState) -> Result<usize, HandlerError> {
    let db = state.db();
    let retention_days = state.config().exports.retention_days;
    if retention_days <= 0 {
        return Ok(0);
    }
    let cutoff = lorehaven_db::identity::in_seconds(-retention_days * 24 * 60 * 60);
    let store = BlobStore::new(state.config().storage.root.clone());

    let expired = repo::purge_expired(db, &cutoff, 500)
        .await
        .map_err(transient)?;
    let mut removed = 0_usize;
    for purged in expired {
        let export_id = purged.id.clone();
        if let Some(checksum) = purged.output_blob_checksum.clone() {
            store
                .unreference(db, &checksum, EXPORT_OWNER_TYPE, &export_id)
                .await
                .map_err(transient)?;
            store
                .delete_if_unreferenced(db, &checksum)
                .await
                .map_err(transient)?;
        }
        removed += 1;
    }

    // `purge_expired` deletes the rows; this collects the grants they leave
    // behind, which are unreachable once their export is gone.
    repo::purge_expired_grants(db, &lorehaven_db::identity::now_rfc3339())
        .await
        .map_err(transient)?;
    Ok(removed)
}


// ---------------------------------------------------------------------------
// CTA injection (spec §42)
// ---------------------------------------------------------------------------

/// Apply the instance CTA to export options unless the work is exempt (§42.2).
/// Evaluated on every export from live marks — never cached — so a retraction
/// takes effect on the next export.
async fn apply_instance_cta(
    state: &AppState,
    work_id: &str,
    options: &mut ExportOptions,
) -> Result<(), ExportFailure> {
    let config = state.config();
    let placement = match config.exports.cta_placement.as_str() {
        "per_chapter" => CtaPlacementSerde::PerChapter,
        "per_work" => CtaPlacementSerde::PerWork,
        "off" => CtaPlacementSerde::Off,
        _ => return Err(ExportFailure::Storage("invalid cta_placement config".to_owned())),
    };
    let quorum = config.exports.cta_quorum as usize;

    let exempt = cta_exemption(state.db(), work_id, quorum)
        .await
        .map_err(|e| ExportFailure::Storage(e.to_string()))?;

    if !exempt && !matches!(placement, CtaPlacementSerde::Off) {
        options.instance_cta = Some(InstanceCta {
            placement,
            html: config.exports.cta_html.clone(),
        });
    }
    Ok(())
}
