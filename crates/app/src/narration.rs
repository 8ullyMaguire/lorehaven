//! TTS narration worker logic (M26 / spec §32.5).
//!
//! The `JobKind::Narration` handler chunks a work's body, synthesizes each
//! chunk via the instance's configured `TtsEngine`, splices the results into
//! one audio artifact, stores it as a blob plus a `media_file` row attached to
//! the narration edition, and records the checksum on the edition.
//!
//! # What this handler deliberately does not do
//!
//! It does not publish the edition. The edition stays in `draft` until the
//! author approves it, which is the §32.5 acceptance rule ("author-approved TTS
//! narration") and the §22.6 machine-producer rule: a machine's output is
//! labeled as such and is never the thing that goes live by itself. The label
//! and the creator credit written by the route are what carry that label; this
//! handler only fills in audio.
//!
//! # Failure classification
//!
//! A wrong or missing edition is *fatal* (retrying cannot conjure it). A
//! missing engine is fatal too, and it is the case the plan calls out: an
//! instance without Piper installed must fail the job with a sentence naming
//! what to install, not crash the worker. A failure mid-synthesis is transient
//! so the retry policy gets its attempts — a job that already stored a blob is
//! idempotent by checksum, so a retry costs a re-synthesis, not a duplicate.

use crate::state::AppState;
use crate::tts::{self, VoiceSpec};
use crate::worker::HandlerError;
use lorehaven_domain::JobId;
use tracing::info;

/// The narration chunk size in characters. Piper handles a few thousand
/// characters well; keep chunks small enough that a single failure loses little
/// work and the worker can checkpoint.
const CHUNK_SIZE: usize = 2_000;

/// The owner type for blob references created by the narration worker.
pub const NARRATION_OWNER_TYPE: &str = "narration";

/// Handle a Narration job: synthesize audio for a narration edition.
///
/// The payload is the narration edition id. See the module docs for what the
/// handler does and does not do.
pub async fn handle_narration(
    state: &AppState,
    job: JobId,
    edition_id: &str,
) -> Result<(), HandlerError> {
    let db = state.db();

    // 1. Load the edition and confirm it is one this handler may fill in.
    let edition = lorehaven_db::media::find_media_edition(db, edition_id)
        .await
        .map_err(transient)?
        .ok_or_else(|| HandlerError::Fatal(format!("narration edition {edition_id} not found")))?;

    if edition.edition_kind != "narration" {
        return Err(HandlerError::Fatal(format!(
            "edition {edition_id} is not a narration edition (kind: {})",
            edition.edition_kind
        )));
    }

    // 2. Collect the work's text, chapter by chapter.
    let work_id: lorehaven_domain::WorkId = edition.work_id.parse().map_err(|_| {
        HandlerError::Fatal(format!(
            "narration edition {edition_id} holds an invalid work id"
        ))
    })?;
    let chapters = lorehaven_db::content::chapters_for_work(db, work_id)
        .await
        .map_err(transient)?;

    let mut full_text = String::new();
    for chapter in &chapters {
        let text = chapter.plain_text.trim();
        if text.is_empty() {
            continue;
        }
        if !full_text.is_empty() {
            full_text.push_str("\n\n");
        }
        full_text.push_str(text);
    }
    if full_text.trim().is_empty() {
        return Err(HandlerError::Fatal(format!(
            "work {} has no text to narrate",
            edition.work_id
        )));
    }

    // 3. Resolve the engine. A name this build does not know, or a binary that
    //    is not installed, is fatal and says so.
    let engine = state.tts_engine().map_err(|reason| {
        HandlerError::Fatal(format!("no narration engine on this instance: {reason}"))
    })?;
    engine.health().map_err(|error| {
        HandlerError::Fatal(format!(
            "narration engine '{}' is not usable: {error}",
            engine.name()
        ))
    })?;

    // 4. Chunk, synthesize, splice.
    let voice = VoiceSpec {
        voice: state.config().tts.default_voice.clone(),
        rate: None,
        sample_rate: None,
    };
    let chunks = chunk_text(&full_text, CHUNK_SIZE);
    let mut parts = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        if lorehaven_db::jobs::is_cancelled(db, job)
            .await
            .map_err(transient)?
        {
            return Err(HandlerError::Cancelled);
        }
        let audio = engine.synthesize(chunk, &voice).map_err(transient)?;
        parts.push(audio);
        let permille = ((index + 1) as i64 * 1000) / chunks.len() as i64;
        lorehaven_db::jobs::progress(
            db,
            job,
            permille,
            Some(&format!(
                "narrated {} of {} passages",
                index + 1,
                chunks.len()
            )),
        )
        .await
        .map_err(transient)?;
    }

    let artifact = tts::concat_audio(&parts).map_err(transient)?;
    if artifact.bytes.is_empty() {
        return Err(HandlerError::Fatal(
            "the narration engine produced no audio".to_owned(),
        ));
    }

    // 5. Store the audio: blob first, then the reference, then the row.
    let store = lorehaven_db::storage::BlobStore::new(state.config().storage.root.clone());
    let (checksum, _key) = store
        .put(db, &artifact.bytes, &artifact.media_type)
        .await
        .map_err(transient)?;
    store
        .reference(db, &checksum, NARRATION_OWNER_TYPE, edition_id)
        .await
        .map_err(transient)?;
    lorehaven_db::media::create_media_file(
        db,
        &edition.work_id,
        "narration",
        &checksum,
        i64::try_from(artifact.bytes.len()).unwrap_or(i64::MAX),
        &artifact.media_type,
    )
    .await
    .map_err(transient)?;

    // 6. Record the checksum on the edition. It stays in draft: the author
    //    approves it, and the replace-with-a-recording flow diffs on this
    //    checksum rather than on the rows existing.
    lorehaven_db::narration::mark_narration_audio_stored(db, edition_id, &checksum)
        .await
        .map_err(transient)?;

    info!(
        edition = edition_id,
        engine = engine.name(),
        bytes = artifact.bytes.len(),
        media_type = %artifact.media_type,
        passages = chunks.len(),
        "narration audio stored; the edition remains a draft until its author approves it"
    );
    Ok(())
}

/// Classify an infrastructure failure: worth retrying.
fn transient(error: impl std::fmt::Display) -> HandlerError {
    HandlerError::Transient(error.to_string())
}

/// Chunk text into pieces no larger than `max_chars` bytes, preferring sentence
/// boundaries so the engine gets natural phrasing.
///
/// The invariant callers rely on is that concatenating the chunks reproduces the
/// input exactly and that every chunk is at most `max_chars` bytes. A
/// multi-byte character is never split, because every candidate position comes
/// from a `char_indices` boundary.
fn chunk_text(text: &str, max_chars: usize) -> Vec<&str> {
    debug_assert!(max_chars > 0);
    let mut chunks = Vec::new();
    let mut rest = text;

    while rest.len() > max_chars {
        // `max_chars` is a byte count, so it can land inside a multi-byte
        // character; back the window edge up to a char boundary or the slice
        // itself would panic before any split logic runs.
        let mut window_len = max_chars.min(rest.len());
        while !rest.is_char_boundary(window_len) {
            window_len -= 1;
        }
        let window = &rest[..window_len];
        // The last sentence-ending character in the window, then the last
        // space, then the hard limit. The fallbacks matter: a single long
        // paragraph with no full stop reaches the hard limit only.
        let split_at = window
            .char_indices()
            .rev()
            .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
            .or_else(|| window.char_indices().rev().find(|(_, c)| c.is_whitespace()))
            .map(|(index, c)| index + c.len_utf8())
            .unwrap_or(if window_len == 0 {
                // Not even one character fits: take the character whole,
                // because a character is the smallest unit that can be spoken.
                rest.chars().next().map_or(1, char::len_utf8)
            } else {
                window_len
            });
        let (chunk, tail) = rest.split_at(split_at);
        chunks.push(chunk);
        rest = tail;
    }
    if !rest.is_empty() {
        chunks.push(rest);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The invariant every caller depends on: nothing is lost, nothing is
    /// duplicated, and no chunk exceeds the limit.
    fn chunk_round_trips(text: &str, max: usize) -> Vec<&str> {
        let chunks = chunk_text(text, max);
        assert_eq!(
            chunks.concat(),
            text,
            "chunking must reproduce the input exactly"
        );
        for chunk in &chunks {
            assert!(
                chunk.len() <= max,
                "chunk longer than the limit: {} > {max}",
                chunk.len()
            );
        }
        chunks
    }

    #[test]
    fn chunk_text_splits_at_sentence_boundaries() {
        let text = "First sentence. Second sentence! Third one? Fourth here.";
        let chunks = chunk_round_trips(text, 30);
        // The `!` sits at byte 31, just outside the 30-byte window, so the
        // first split falls back to the last sentence end inside the window:
        // the `.` that closes "First sentence".
        assert_eq!(chunks[0], "First sentence.");
        assert_eq!(
            chunks,
            vec![
                "First sentence.",
                " Second sentence! Third one?",
                " Fourth here."
            ]
        );
    }

    #[test]
    fn chunk_text_single_chunk_when_short() {
        let text = "A short text.";
        let chunks = chunk_round_trips(text, 100);
        assert_eq!(chunks, vec!["A short text."]);
    }

    #[test]
    fn chunk_text_falls_back_to_whitespace_without_sentences() {
        // A long run with spaces and no terminal punctuation: breaking mid-word
        // would be audible, so the split goes to the last space in the window.
        let text = "aaaaa bbbbb ccccc ddddd eeeee";
        let chunks = chunk_round_trips(text, 12);
        assert!(chunks.len() > 1);
        assert!(
            chunks[0].ends_with(' '),
            "the first split should be after a space: {:?}",
            chunks[0]
        );
    }

    #[test]
    fn chunk_text_hard_splits_unbreakable_text() {
        // No whitespace and no punctuation: the limit has to win, or the engine
        // would be handed the whole work as one passage.
        let text = "x".repeat(50);
        let chunks = chunk_round_trips(&text, 10);
        assert_eq!(chunks.len(), 5);
        assert!(chunks.iter().all(|chunk| chunk.len() == 10));
    }

    #[test]
    fn chunk_text_never_splits_a_multibyte_character() {
        // Every character here is four bytes, so a byte-index split would panic
        // on a non-boundary. The round trip is the proof that it did not.
        let text = "🙂".repeat(30);
        let chunks = chunk_round_trips(&text, 11);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn chunk_text_on_empty_text_yields_nothing() {
        assert!(chunk_text("", CHUNK_SIZE).is_empty());
    }

    #[test]
    fn chunk_text_leaves_text_of_exactly_the_limit_whole() {
        let text = "y".repeat(CHUNK_SIZE);
        let chunks = chunk_round_trips(&text, CHUNK_SIZE);
        assert_eq!(chunks.len(), 1);
    }
}
