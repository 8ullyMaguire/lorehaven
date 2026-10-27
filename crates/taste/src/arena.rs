//! The arena (spec §12.4).
//!
//! The ladder behind the pairwise comparison surface: two works, a voter picks one, neither,
//! or the pair is skipped, and the works acquire a rating from those judgements.
//!
//! **The ladder is now real; the surface is not yet.** [`glicko`] is the rating algorithm —
//! Glicko-2 with draws, ported from kindred's reference implementation and pinned to
//! Glickman's worked example — and it is what the archive's arena should rank with. The
//! pairing, the voter weighting and the surface that feeds it are still to be built, and
//! they stay absent rather than stubbed: a pairing that returns a plausible pair is a
//! measurement that looks taken and is not.

/// Glicko-2 with draws: the rating algorithm, its constants and its worked example.
pub mod glicko;
