//! An account's institutional standing, read from the database (spec §7.7).
//!
//! The domain's `can_access_content_with_standing` is pure: it takes the facts
//! and answers. This module is the one place that *looks them up*, so the four
//! facts come from four different tables with four different lifecycles and the
//! rule for combining them is written down once rather than at every call site.
//!
//! Four separate queries, on purpose. Trust, operator role, vanguard membership
//! and curator opt-in live in four tables with four different rules — the
//! vanguard row expires, the curator row can be opted out of, the operator row
//! is a plain grant and the trust level is recomputed. A single `SELECT` with
//! four `LEFT JOIN`s would be faster and would quietly have to decide what
//! happens when one of them is missing, and every one of those four answers
//! turns out to be "no, and that is a false, not an error" — which is precisely
//! the thing a join gets wrong by returning a row of nulls.

use crate::Database;
use lorehaven_domain::policy::ActorStanding;

/// Read one account's standing.
///
/// A database error fails the whole lookup rather than degrading to
/// `ActorStanding::none()`. That is the difference between a reader being
/// refused a body they qualify for and a reader being refused one they do not:
/// the first is a visible bug, the second is a correct answer to a different
/// question, and silently returning "no standing" would produce the second
/// every time the first happened. Callers that genuinely want a best-effort
/// answer pass `None` for `account` and get `none()` without touching the
/// database.
pub async fn standing_for(
    db: &Database,
    account_id: Option<&str>,
) -> Result<ActorStanding, sqlx::Error> {
    let Some(account) = account_id else {
        return Ok(ActorStanding::none());
    };

    // Sequential rather than joined, and the ordering is not meaningful — each
    // lookup is independent and none can veto another. `try_join!` would read
    // better and would lose the ability to say which query failed.
    let trust_level = crate::governance::trust_for(db, account).await?;
    let is_operator = crate::governance::has_operator_role(db, account).await?;
    let is_vanguard = crate::roles::is_vanguard(db, account).await?;
    let is_curator = crate::media_resilience::is_active_curator(db, account).await?;

    Ok(ActorStanding {
        trust_level,
        is_operator,
        is_vanguard,
        is_curator,
        // A caller that got here with an account has, by construction, a
        // session. This is the fact that separates `accounts_only` from
        // `trust_at_least:0`, and it is the one field no table holds: a session
        // is a request property, not a stored one.
        signed_in: true,
    })
}
