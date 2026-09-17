//! TTS narration engine abstraction (M26 / spec §32.5).
//!
//! A `TtsEngine` produces audio bytes from text. Engines are stateless over
//! the worker job; the handler chunks the work body, synthesizes per chunk,
//! concatenates, and stores the bytes as a `media_file` row attached to the
//! narration edition.
//!
//! The built-in default is Piper (local). Cloud adapters (ElevenLabs, AWS
//! Polly) implement the same trait behind per-instance API keys and budget
//! guards — see `crate::config::TtsConfig`.
//!
//! # Why the engine is chosen once, by name
//!
//! `tts.engine` names the engine and [`build_engine`] is the only place that
//! maps a name to an implementation. An instance without the binary installed
//! gets an engine whose [`TtsEngine::health`] fails with a sentence naming the
//! missing program, so the worker fails the job with that sentence rather than
//! panicking on a missing file — which is also what `lorehaven doctor`
//! reports before a reader ever queues one.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::TtsConfig;

/// A voice specification. Engines map this to their own model/voice naming.
#[derive(Debug, Clone, Default)]
pub struct VoiceSpec {
    /// The voice or model name (e.g. "en_US-lessac-medium" for Piper).
    pub voice: Option<String>,
    /// Speech rate multiplier (1.0 = engine default).
    pub rate: Option<f64>,
    /// Output sample rate in Hz (engine may ignore).
    pub sample_rate: Option<u32>,
}

/// The result of a synthesis: raw audio bytes plus the media type.
#[derive(Debug, Clone)]
pub struct Audio {
    pub bytes: Vec<u8>,
    pub media_type: String,
}

/// A TTS engine. Implemented by Piper (local) and cloud adapters.
///
/// Engines are `Send + Sync` because they're held in `AppState` and invoked
/// from worker tasks.
pub trait TtsEngine: Send + Sync {
    /// The engine's display name for logs and the edition label.
    fn name(&self) -> &str;

    /// Whether the engine is available (binary installed / API key set).
    /// Returns false cleanly rather than erroring — the worker uses this to
    /// fail the job with a clear "engine not installed" message.
    fn is_available(&self) -> bool;

    /// A health probe. Returns Ok(()) if the engine can currently produce
    /// audio, Err otherwise. Used by the `/health/ready` gate and the worker.
    fn health(&self) -> Result<()>;

    /// Synthesize `text` into audio. The caller chunks the work body; each
    /// call is independent and the results are concatenated.
    fn synthesize(&self, text: &str, voice: &VoiceSpec) -> Result<Audio>;
}

/// A trait object alias for brevity in state and function signatures.
pub type DynTtsEngine = Box<dyn TtsEngine>;

/// The engines this build can name in `tts.engine`.
pub const SUPPORTED_ENGINES: [&str; 2] = ["piper", "silent"];

/// Build the configured [`TtsEngine`].
///
/// `piper_on_path` is what converter discovery found (`which piper`), passed in
/// rather than looked up here so an instance's answer is the same everywhere
/// (`state.converters()` owns detection). A configured `tts.piper_path` wins
/// over the discovered one: an operator who names a binary means that binary.
pub fn build_engine(config: &TtsConfig, piper_on_path: Option<&Path>) -> Result<DynTtsEngine> {
    match config.engine.as_str() {
        "piper" => {
            let program = config
                .piper_path
                .clone()
                .or_else(|| piper_on_path.map(Path::to_path_buf));
            let model = config
                .piper_voice_model
                .clone()
                .or_else(|| voice_model_from_default(config.default_voice.as_deref()));
            match (program, model) {
                (Some(program), Some(model)) => Ok(Box::new(PiperEngine::new(&program, &model))),
                (None, _) => Ok(Box::new(MissingEngine::new(
                    "piper",
                    "the `piper` binary is not on PATH and `tts.piper_path` is unset",
                ))),
                (_, None) => Ok(Box::new(MissingEngine::new(
                    "piper",
                    "no voice model: set `tts.piper_voice_model` to a Piper `.onnx` file",
                ))),
            }
        }
        // A silent engine that produces a valid WAV. It exists so the whole
        // narration pipeline — chunking, concatenation, the blob and media_file
        // rows, the draft gate — can be exercised on an instance that has no
        // synthesizer at all, including in CI.
        "silent" => Ok(Box::new(SilentEngine::new())),
        other => anyhow::bail!(
            "unknown tts.engine {other:?}; supported engines are {}",
            SUPPORTED_ENGINES.join(", ")
        ),
    }
}

/// Resolve a bare voice name to a model path when it is already a path.
///
/// `tts.default_voice` may name a voice (`en_US-lessac-medium`) or a file. A
/// name is not resolvable here because Piper's model directory is a property of
/// the installation, so only a path is honoured; a bare name gets the honest
/// "no voice model" failure.
fn voice_model_from_default(default_voice: Option<&str>) -> Option<PathBuf> {
    let voice = default_voice?;
    let path = PathBuf::from(voice);
    path.exists().then_some(path)
}

// ---------------------------------------------------------------------------
// Audio concatenation
// ---------------------------------------------------------------------------

/// Concatenate per-chunk synthesis results into one artifact.
///
/// WAV parts are spliced sample-wise rather than joined byte-wise: a WAV file
/// begins with a 44-byte header and a length field, so `[a, b].concat()` is not
/// a playable file. Everything else (an engine that returns a self-delimiting
/// format) is concatenated as-is, which is what those formats are for.
pub fn concat_audio(parts: &[Audio]) -> Result<Audio> {
    let first = parts
        .first()
        .ok_or_else(|| anyhow::anyhow!("nothing was synthesized"))?;
    if parts.iter().any(|p| p.media_type != first.media_type) {
        anyhow::bail!("cannot concatenate audio of mixed media types");
    }
    if first.media_type == "audio/wav" {
        let wav_parts = parts
            .iter()
            .filter(|p| !p.bytes.is_empty())
            .map(|p| p.bytes.as_slice());
        return Ok(Audio {
            bytes: concat_wav(wav_parts)?,
            media_type: first.media_type.clone(),
        });
    }
    let mut bytes = Vec::with_capacity(parts.iter().map(|p| p.bytes.len()).sum());
    for part in parts {
        bytes.extend_from_slice(&part.bytes);
    }
    Ok(Audio {
        bytes,
        media_type: first.media_type.clone(),
    })
}

/// A parsed RIFF/WAVE header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WavHeader {
    /// Total bytes of the RIFF chunk, header included.
    riff_size: u32,
    /// Byte offset of the `fmt ` chunk's payload.
    fmt_offset: usize,
    /// Byte offset where the `data` payload starts.
    data_offset: usize,
    /// Byte length of the `data` payload.
    data_len: u32,
}

/// Splice the `data` payloads of several WAV files into one WAV.
///
/// The first part supplies the header (including any chunks such as `LIST` that
/// sit before `data`); every part's samples are appended to it and the RIFF and
/// `data` length fields are rewritten. Only the layout the engines here emit is
/// accepted (a `fmt ` chunk before a `data` chunk). Anything else is refused
/// rather than guessed at: producing a file that plays as noise is worse than a
/// failed job.
fn concat_wav<'a>(parts: impl Iterator<Item = &'a [u8]>) -> Result<Vec<u8>> {
    let mut template: Option<Vec<u8>> = None;
    let mut format: Option<Vec<u8>> = None;
    let mut payload: Vec<u8> = Vec::new();

    for bytes in parts {
        let header = parse_wav(bytes)?;
        let this_format = bytes[header.fmt_offset..header.data_offset].to_vec();
        match &format {
            // The `fmt ` chunk encodes rate, channels, depth and codec, so byte
            // equality is exactly the condition under which payloads may join.
            Some(known) if known != &this_format => {
                anyhow::bail!("cannot splice WAV chunks with different audio formats")
            }
            Some(_) => {}
            None => format = Some(this_format),
        }
        if template.is_none() {
            template = Some(bytes[..header.data_offset].to_vec());
        }
        let end = header.data_offset + header.data_len as usize;
        if payload.len() + header.data_len as usize > u32::MAX as usize {
            anyhow::bail!("concatenated audio exceeds the WAV 4 GiB limit");
        }
        payload.extend_from_slice(&bytes[header.data_offset..end]);
    }

    let mut out = template.ok_or_else(|| anyhow::anyhow!("no audio to write"))?;
    let data_offset = out.len();
    out.extend_from_slice(&payload);
    rewrite_wav_sizes(&mut out, data_offset, payload.len())?;
    Ok(out)
}

/// Parse the canonical WAV layout (a `fmt ` chunk before a `data` chunk).
fn parse_wav(bytes: &[u8]) -> Result<WavHeader> {
    if bytes.len() < 12 {
        anyhow::bail!("not a WAV file: shorter than the RIFF header");
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        anyhow::bail!("not a WAV file: missing the RIFF/WAVE signature");
    }
    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let mut cursor = 12usize;
    let mut fmt_offset = None;
    let mut data_offset = None;
    let mut data_len = 0u32;
    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        let payload_start = cursor + 8;
        let payload_end = payload_start
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("malformed WAV chunk length"))?;
        if payload_end > bytes.len() {
            anyhow::bail!("malformed WAV: a chunk runs past the end of the file");
        }
        if id == b"fmt " && fmt_offset.is_none() {
            fmt_offset = Some(payload_start);
        } else if id == b"data" && data_offset.is_none() {
            data_offset = Some(payload_start);
            data_len = size as u32;
        }
        if fmt_offset.is_some() && data_offset.is_some() {
            break;
        }
        // RIFF chunks are word-aligned.
        cursor = payload_end + (size % 2);
    }
    Ok(WavHeader {
        riff_size,
        fmt_offset: fmt_offset.ok_or_else(|| anyhow::anyhow!("WAV has no fmt chunk"))?,
        data_offset: data_offset.ok_or_else(|| anyhow::anyhow!("WAV has no data chunk"))?,
        data_len,
    })
}

/// Rewrite the RIFF size and the `data` chunk size after splicing.
fn rewrite_wav_sizes(buffer: &mut [u8], data_offset: usize, data_len: usize) -> Result<()> {
    let data_len = u32::try_from(data_len).map_err(|_| anyhow::anyhow!("audio too large"))?;
    let riff_size = u32::try_from(buffer.len() - 8)
        .map_err(|_| anyhow::anyhow!("audio too large for a WAV container"))?;
    buffer[4..8].copy_from_slice(&riff_size.to_le_bytes());
    // The `data` chunk size field sits immediately before its payload.
    let size_field = data_offset - 4;
    buffer[size_field..size_field + 4].copy_from_slice(&data_len.to_le_bytes());
    Ok(())
}

/// Build a canonical 44-byte-header WAV from raw PCM samples.
///
/// Used by the silent engine and by tests, which need a well-formed container
/// without a synthesizer.
#[must_use]
pub fn wav_from_pcm(pcm: &[u8], sample_rate: u32, channels: u16, bits_per_sample: u16) -> Vec<u8> {
    let block_align = channels * bits_per_sample / 8;
    let byte_rate = sample_rate * u32::from(block_align);
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

// ---------------------------------------------------------------------------
// Piper adapter (local, default)
// ---------------------------------------------------------------------------

/// A Piper TTS engine. Piper runs as a local binary: `piper --model <path>
/// --output_file <path>` reads text from stdin and writes a WAV file.
pub struct PiperEngine {
    program: PathBuf,
    model: PathBuf,
}

impl PiperEngine {
    pub fn new(program: &Path, model: &Path) -> Self {
        Self {
            program: program.to_path_buf(),
            model: model.to_path_buf(),
        }
    }
}

impl TtsEngine for PiperEngine {
    fn name(&self) -> &str {
        "piper"
    }

    fn is_available(&self) -> bool {
        self.program.exists() && self.model.exists()
    }

    fn health(&self) -> Result<()> {
        if !self.program.exists() {
            anyhow::bail!("piper binary not found at {}", self.program.display());
        }
        if !self.model.exists() {
            anyhow::bail!("piper voice model not found at {}", self.model.display());
        }
        Ok(())
    }

    fn synthesize(&self, text: &str, voice: &VoiceSpec) -> Result<Audio> {
        let temp_dir = std::env::temp_dir().join(format!("lorehaven-tts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir)?;
        let output_path = temp_dir.join("output.wav");

        let mut cmd = std::process::Command::new(&self.program);
        cmd.arg("--model")
            .arg(&self.model)
            .arg("--output_file")
            .arg(&output_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped());

        if let Some(rate) = voice.rate {
            // Piper's `--length_scale` is the reciprocal of a rate multiplier:
            // 1.5 asks the voice to speak 1.5× slower.
            if rate > 0.0 {
                cmd.arg("--length_scale").arg((1.0 / rate).to_string());
            }
        }
        if let Some(sample_rate) = voice.sample_rate {
            cmd.arg("--sample_rate").arg(sample_rate.to_string());
        }
        if let Some(voice_name) = &voice.voice {
            // Only a model path is meaningful without Piper's model directory.
            let candidate = PathBuf::from(voice_name);
            if candidate.exists() {
                cmd.arg("--model").arg(candidate);
            }
        }

        let mut child = cmd.spawn().with_context(|| "spawning piper")?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(text.as_bytes())?;
        }
        let output = child.wait_with_output()?;

        if !output.status.success() {
            anyhow::bail!("piper failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        let bytes = std::fs::read(&output_path)
            .with_context(|| format!("reading piper output at {}", output_path.display()))?;

        let _ = std::fs::remove_dir_all(&temp_dir);

        Ok(Audio {
            bytes,
            media_type: "audio/wav".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Silent engine (no synthesizer required)
// ---------------------------------------------------------------------------

/// An engine that emits silence of a length proportional to the text.
///
/// Chosen with `tts.engine = "silent"`. It is not a narrator — the label and
/// the machine-producer credit still say so — but it lets an operator verify
/// the whole pipeline (chunking, splicing, storage, the draft gate) on a host
/// with no TTS at all, and it is what the pipeline tests run against in CI.
pub struct SilentEngine {
    sample_rate: u32,
}

impl Default for SilentEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SilentEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sample_rate: 22_050,
        }
    }
}

impl TtsEngine for SilentEngine {
    fn name(&self) -> &str {
        "silent"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn health(&self) -> Result<()> {
        Ok(())
    }

    fn synthesize(&self, text: &str, _voice: &VoiceSpec) -> Result<Audio> {
        // ~10 ms per character, 16-bit mono at the engine's rate: enough that
        // the length of the input is visible in the output, so a caller can
        // tell a spliced artifact from a single chunk.
        let chars = text.chars().count() as u32;
        let samples = chars.saturating_mul(self.sample_rate / 100).max(1);
        let pcm = vec![0u8; samples as usize * 2];
        Ok(Audio {
            bytes: wav_from_pcm(&pcm, self.sample_rate, 1, 16),
            media_type: "audio/wav".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Missing engine (installed nothing)
// ---------------------------------------------------------------------------

/// An engine whose `health()` names exactly what is missing.
///
/// It exists so that "not installed" is a *reported* condition rather than a
/// panic or a silent empty file: the worker turns it into a fatal job failure
/// carrying the same sentence `lorehaven doctor` prints.
pub struct MissingEngine {
    name: &'static str,
    reason: String,
}

impl MissingEngine {
    #[must_use]
    pub fn new(name: &'static str, reason: impl Into<String>) -> Self {
        Self {
            name,
            reason: reason.into(),
        }
    }
}

impl TtsEngine for MissingEngine {
    fn name(&self) -> &str {
        self.name
    }

    fn is_available(&self) -> bool {
        false
    }

    fn health(&self) -> Result<()> {
        anyhow::bail!("{} is not available: {}", self.name, self.reason)
    }

    fn synthesize(&self, _text: &str, _voice: &VoiceSpec) -> Result<Audio> {
        self.health()?;
        unreachable!("health() always fails for a missing engine")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm_wav(samples: &[u8], rate: u32) -> Audio {
        Audio {
            bytes: wav_from_pcm(samples, rate, 1, 16),
            media_type: "audio/wav".to_string(),
        }
    }

    /// `build_engine`'s `Ok` is a trait object that is not `Debug`, so
    /// `expect_err` cannot be used on it; this reads the error out by hand.
    fn build_error(config: &TtsConfig, piper_on_path: Option<&Path>) -> String {
        match build_engine(config, piper_on_path) {
            Ok(engine) => panic!("expected a refusal, got engine {:?}", engine.name()),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn wav_from_pcm_has_a_canonical_header() {
        let wav = wav_from_pcm(&[1, 2, 3, 4], 44_100, 2, 16);
        let header = parse_wav(&wav).expect("parses");
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(header.data_len, 4);
        assert_eq!(header.data_offset, 44);
        assert_eq!(header.riff_size as usize, wav.len() - 8);
        // The sample rate is at offset 24 (12 + 8 chunk header + 4 fields).
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            44_100
        );
    }

    #[test]
    fn concat_splices_wav_payloads_and_fixes_sizes() {
        // Two chunks of two samples each: four samples' worth must come out,
        // and the header must describe the spliced length, not the first part's.
        let a = pcm_wav(&[1, 2, 3, 4], 22_050);
        let b = pcm_wav(&[5, 6, 7, 8], 22_050);
        let joined = concat_audio(&[a, b]).expect("splices");
        let header = parse_wav(&joined.bytes).expect("result parses");
        assert_eq!(header.data_len, 8, "both payloads are present");
        assert_eq!(header.data_offset, 44, "only one header");
        assert_eq!(
            &joined.bytes[header.data_offset..header.data_offset + 8],
            &[1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert_eq!(header.riff_size as usize, joined.bytes.len() - 8);
        assert_eq!(joined.media_type, "audio/wav");
    }

    #[test]
    fn concat_refuses_mismatched_formats() {
        let a = pcm_wav(&[0, 0], 22_050);
        let b = pcm_wav(&[0, 0], 44_100);
        let error = concat_audio(&[a, b]).expect_err("must refuse");
        assert!(
            error.to_string().contains("different audio formats"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn concat_refuses_mixed_media_types() {
        let a = pcm_wav(&[0, 0], 22_050);
        let b = Audio {
            bytes: vec![0, 0],
            media_type: "audio/mpeg".into(),
        };
        assert!(concat_audio(&[a, b]).is_err());
    }

    #[test]
    fn concat_passes_through_a_single_part_unchanged() {
        let a = pcm_wav(&[9, 9], 8_000);
        let joined = concat_audio(std::slice::from_ref(&a)).expect("splices");
        assert_eq!(joined.bytes, a.bytes);
    }

    #[test]
    fn concat_ignores_empty_parts() {
        let a = pcm_wav(&[1, 2], 8_000);
        let empty = Audio {
            bytes: Vec::new(),
            media_type: "audio/wav".into(),
        };
        let joined = concat_audio(&[a, empty]).expect("splices");
        let header = parse_wav(&joined.bytes).expect("parses");
        assert_eq!(header.data_len, 2);
    }

    #[test]
    fn concat_rejects_non_audio_bytes() {
        let a = Audio {
            bytes: b"not a wav".to_vec(),
            media_type: "audio/wav".into(),
        };
        assert!(concat_audio(&[a]).is_err());
    }

    #[test]
    fn silent_engine_emits_parseable_wav_whose_length_follows_the_text() {
        let engine = SilentEngine::new();
        let short = engine.synthesize("hi", &VoiceSpec::default()).unwrap();
        let long = engine
            .synthesize("a much longer sentence to narrate", &VoiceSpec::default())
            .unwrap();
        assert_eq!(short.media_type, "audio/wav");
        parse_wav(&short.bytes).expect("short parses");
        assert!(long.bytes.len() > short.bytes.len());
        assert!(engine.is_available());
    }

    #[test]
    fn build_engine_selects_the_configured_engine() {
        let config = TtsConfig {
            engine: "silent".into(),
            ..TtsConfig::default()
        };
        let engine = build_engine(&config, None).expect("silent always builds");
        assert_eq!(engine.name(), "silent");
        assert!(engine.health().is_ok());
    }

    #[test]
    fn build_engine_reports_an_unknown_name_with_the_supported_list() {
        let config = TtsConfig {
            engine: "elevenlabs".into(),
            ..TtsConfig::default()
        };
        let message = build_error(&config, None);
        assert!(message.contains("elevenlabs"), "{message}");
        assert!(message.contains("piper"), "{message}");
    }

    #[test]
    fn piper_without_an_installation_is_unavailable_but_named() {
        let config = TtsConfig {
            engine: "piper".into(),
            ..TtsConfig::default()
        };
        let engine = build_engine(&config, None).expect("builds a reporting engine");
        assert_eq!(engine.name(), "piper");
        assert!(!engine.is_available());
        let error = engine.health().expect_err("health names the gap");
        assert!(error.to_string().contains("PATH"), "{error}");
        assert!(engine.synthesize("text", &VoiceSpec::default()).is_err());
    }

    #[test]
    fn a_configured_piper_path_wins_over_path_discovery() {
        // A path that exists (the test binary itself): the engine must be built
        // from the configured program, which is what `is_available` reports.
        let configured = std::env::current_exe().expect("test binary path");
        let config = TtsConfig {
            engine: "piper".into(),
            piper_path: Some(configured.clone()),
            piper_voice_model: Some(configured),
            default_voice: None,
            monthly_spend_cap_cents: None,
        };
        let engine = build_engine(&config, Some(Path::new("/nonexistent/piper"))).expect("builds");
        assert!(engine.is_available());
    }

    #[test]
    fn a_voice_name_that_is_not_a_path_is_not_a_model() {
        // A bare voice name is not a model path: Piper's model directory is a
        // property of an installation, so the honest answer is "no voice model"
        // rather than a guessed filename. The program is supplied so this
        // asserts the missing *model* and not the missing binary, which is what
        // it would otherwise report first.
        let configured = std::env::current_exe().expect("test binary path");
        let config = TtsConfig {
            engine: "piper".into(),
            piper_path: Some(configured),
            default_voice: Some("en_US-lessac-medium".into()),
            ..TtsConfig::default()
        };
        let engine = build_engine(&config, None).expect("builds a reporting engine");
        assert!(!engine.is_available());
        let error = engine.health().expect_err("no model file");
        assert!(error.to_string().contains("voice model"), "{error}");
    }
}
