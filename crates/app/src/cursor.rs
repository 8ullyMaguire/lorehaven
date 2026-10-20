//! The cursor codec, in one place.
//!
//! # Why this exists
//!
//! Three collections needed the same cursor before this file did: the import
//! list, the job list, and the modlog. Two of them had their own copy of the
//! codec, and a third copy would have been a third place where "what a cursor
//! looks like" is decided.
//!
//! **A cursor is a promise.** A client that holds a cursor and hands it back
//! must get the page it was given, and the only way that holds is if the
//! encoding is one thing everywhere. Two codecs that differ by a character
//! would produce two collections whose cursors cannot be exchanged, and a
//! client that guessed wrong would get a validation error rather than a page.
//!
//! # The shape
//!
//! `base64url(created_at | id)` — URL-safe, unpadded, so it survives a query
//! string without escaping.
//!
//! **The pair, not the timestamp alone.** Two rows written in the same
//! millisecond are ordered by the id as well, and a cursor that remembered only
//! the timestamp would make the second page overlap the first.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
use base64::Engine as _;

use crate::http::{ApiError, ApiResult};
use darkhold_domain::AppError;

/// Encode a cursor from the row it points at.
#[must_use]
pub fn encode(created_at: &str, id: &str) -> String {
    BASE64.encode(format!("{created_at}|{id}"))
}

/// Decode a cursor into the row it points at.
///
/// **A cursor this collection did not issue is a validation failure, not an
/// empty page.** A malformed cursor means the client and the server disagree
/// about what is being paged, and answering with an empty page would say "you
/// have reached the end" when the truth is "I did not understand you".
pub fn decode(raw: &str) -> ApiResult<(String, String)> {
    let decoded = BASE64
        .decode(raw)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .ok_or_else(|| {
            ApiError(AppError::field(
                "cursor",
                "is not one this collection issued",
            ))
        })?;
    let (created_at, id) = decoded.split_once('|').ok_or_else(|| {
        ApiError(AppError::field(
            "cursor",
            "is not one this collection issued",
        ))
    })?;
    Ok((created_at.to_owned(), id.to_owned()))
}
