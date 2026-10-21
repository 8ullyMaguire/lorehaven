//! The declared route table.
//!
//! **This is the single declaration of the instance's public surface**, and it
//! lives in the crate rather than in a test for a reason: two tests need it, and
//! a table that exists in two places is a table that can be in one and not the
//! other.
//!
//!   * the **route inventory test** (`crates/app/tests/route_inventory.rs`)
//!     asserts that every registered route has an entry here with the correct
//!     audience, and that every entry here is really registered;
//!   * the **OpenAPI test** (`crates/app/tests/openapi.rs`) asserts that every
//!     public route appears in the published document and that every path in the
//!     document resolves to a registered route.
//!
//! **The order is a claim.** The OpenAPI document is generated from this table
//! (spec §20.4), so a route cannot be published without being declared, and a
//! declared route cannot be published without being documented.

use serde::Serialize;

/// The audience a handler must declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Audience {
    /// Public door — `MaybeSession` (anonymous may reach it).
    Public,
    /// Authenticated door — `RequireSession`.
    Authenticated,
    /// Pseudonymous door — `RequirePseud`.
    Pseudonymous,
    /// Operator door — `RequireSession` + `require_operator`.
    Operator,
    /// Token-or-session door, with no scope requirement — `RequireActor`.
    ///
    /// A distinct kind from `Scoped`, because the difference is exactly the one
    /// that matters for `/me/credential`: a route behind `RequireActor` asks
    /// *who are you* and not *what may you do*. A token with no scopes left must
    /// still be able to discover that it has none — otherwise its only recourse
    /// is a confusing 403 on an unrelated call, and it cannot tell "expired"
    /// from "still fine".
    ///
    /// `RequireActorScoped` is that same question with a scope check bolted on,
    /// so a `Scoped` route answers a token that lacks the scope with 403 and
    /// never reveals the credential exists. Both are needed, and which one a
    /// route uses is a claim about what it discloses, so it is enumerated.
    Actor,
    /// Token-or-session door — `RequireActorScoped`.
    ///
    /// A distinct kind, not a flavour of `Authenticated`, because the two are
    /// not interchangeable: a session's authority was settled at login and
    /// spans whatever that account may do, while a token carries an explicit
    /// scope set and an acting pseud. Collapsing them would let a token-scope
    /// regression hide behind a session test, or the reverse.
    ///
    /// M54-A5 added this for the §23.2 bot actions. It is the *audience* that
    /// is new; which scope a given door wants is checked by
    /// `m54_bot_actions.rs`, the only place that can tell one scope from
    /// another.
    Scoped,
}

impl Audience {
    pub fn as_str(&self) -> &'static str {
        match self {
            Audience::Public => "MaybeSession",
            Audience::Authenticated => "RequireSession",
            Audience::Pseudonymous => "RequirePseud",
            Audience::Operator => "RequireSession",
            Audience::Actor => "RequireActor",
            Audience::Scoped => "RequireActorScoped",
        }
    }
}

/// A single registered route with its expected audience.
///
/// `file` is the route module filename (e.g. `"works.rs"`).
/// `handler` is the function name (e.g. `"list_works"`).
/// `method` is the HTTP method in uppercase (e.g. `"GET"`, `"POST"`).
/// `path` is the route path pattern (e.g. `"/works"`).
/// `audience` is the expected audience extractor.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub file: &'static str,
    pub handler: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub audience: Audience,
}

/// The complete declared set.  Every route the server registers must
/// appear here with its correct audience.  This table is the spec
/// answer to N2: the harness now measures the *right* audience, not
/// just any audience.
///
/// Populated from `server.rs` route registrations and verified
/// against each module's handler signatures.
pub const ROUTE_TABLE: &[RouteEntry] = &[
    // ------------------------------------------------------------------
    // Meta — public read
    // ------------------------------------------------------------------
    RouteEntry {
        file: "meta.rs",
        handler: "meta",
        method: "GET",
        path: "/meta",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Metadata exchange (M57, spec §11.17)
    //
    // All three are `MaybeSession` deliberately, and the reason is the check
    // order rather than anonymity. §11.17 requires a server that did not enable
    // the exchange to be indistinguishable from one without the door, and the
    // opt-in has to be tested *before* the session — otherwise an anonymous
    // caller learns the exchange exists from a 401 where a 404 would have said
    // nothing. So the extractor is the permissive one and the handler does the
    // gating itself, in this order: opt-in (404) → version → session (401) →
    // trust bar (403).
    //
    // Declaring them `Authenticated` would be a lie the test could not detect
    // and a future reader would believe: the extractor would then be
    // `RequireSession`, and the 401 would arrive *before* the 404.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "exchange.rs",
        handler: "get_version",
        method: "GET",
        path: "/exchange/version",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "exchange.rs",
        handler: "submit_signals",
        method: "POST",
        path: "/exchange/signals",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "exchange.rs",
        handler: "get_canonical",
        method: "GET",
        path: "/exchange/canonical",
        audience: Audience::Public,
    },
    // The two governance doors. `MaybeSession` for the same reason as the three
    // above — the opt-in is checked first so a disabled instance answers 404
    // before either the session or the trust bar is consulted. What differs is
    // the check *after*: these refuse below TL3, and that refusal is a governance
    // decision rather than an authentication one, so it is not the extractor's
    // job.
    RouteEntry {
        file: "exchange.rs",
        handler: "get_review_queue",
        method: "GET",
        path: "/exchange/review-queue",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "exchange.rs",
        handler: "curate_entity",
        method: "POST",
        path: "/exchange/entities/curate",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Health — public read
    // ------------------------------------------------------------------
    RouteEntry {
        file: "health.rs",
        handler: "live",
        method: "GET",
        path: "/health/live",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "health.rs",
        handler: "ready",
        method: "GET",
        path: "/health/ready",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Auth — session-scoped (register/login/logout are writes)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "auth.rs",
        handler: "register",
        method: "POST",
        path: "/auth/register",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "login",
        method: "POST",
        path: "/auth/login",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "logout",
        method: "POST",
        path: "/auth/logout",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "password_reset",
        method: "POST",
        path: "/auth/password-reset",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "password_reset_complete",
        method: "POST",
        path: "/auth/password-reset/complete",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "list_sessions",
        method: "GET",
        path: "/auth/sessions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "revoke_all_sessions",
        method: "POST",
        path: "/auth/sessions/revoke-all",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "revoke_session",
        method: "DELETE",
        path: "/auth/sessions/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "auth.rs",
        handler: "me",
        method: "GET",
        path: "/auth/me",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Pseudonyms — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "pseuds.rs",
        handler: "list_pseuds",
        method: "GET",
        path: "/pseuds",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "pseuds.rs",
        handler: "create_pseud",
        method: "POST",
        path: "/pseuds",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "pseuds.rs",
        handler: "update_pseud",
        method: "PATCH",
        path: "/pseuds/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "pseuds.rs",
        handler: "activate_pseud",
        method: "POST",
        path: "/pseuds/{id}/activate",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "pseuds.rs",
        handler: "public_profile",
        method: "GET",
        path: "/pseuds/{id}/profile",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Settings — session-scoped reads and writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "settings.rs",
        handler: "get_privacy",
        method: "GET",
        path: "/settings/privacy",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "patch_privacy",
        method: "PATCH",
        path: "/settings/privacy",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "get_content",
        method: "GET",
        path: "/settings/content",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "patch_content",
        method: "PATCH",
        path: "/settings/content",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "get_typography",
        method: "GET",
        path: "/settings/typography",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "save_typography",
        method: "PATCH",
        path: "/settings/typography",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // M47: User Configuration (spec §46)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "settings.rs",
        handler: "get_search_settings",
        method: "GET",
        path: "/settings/search",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "patch_search_settings",
        method: "PATCH",
        path: "/settings/search",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "delete_search_setting",
        method: "DELETE",
        path: "/settings/search/{key}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "list_content_filters",
        method: "GET",
        path: "/settings/content-filters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "post_content_filter",
        method: "POST",
        path: "/settings/content-filters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "delete_content_filter",
        method: "DELETE",
        path: "/settings/content-filters/{filter_type}/{value}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "get_notification_routes",
        method: "GET",
        path: "/settings/notifications",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "patch_notification_route",
        method: "PATCH",
        path: "/settings/notifications",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "delete_notification_route",
        method: "DELETE",
        path: "/settings/notifications/{event_type}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "export_settings",
        method: "GET",
        path: "/settings/export",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "import_settings",
        method: "POST",
        path: "/settings/import",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Works — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "works.rs",
        handler: "list_works",
        method: "GET",
        path: "/works",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "create_work",
        method: "POST",
        path: "/works",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "update_work",
        method: "PATCH",
        path: "/works/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "publish_work",
        method: "POST",
        path: "/works/{id}/publish",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "withdraw_work",
        method: "POST",
        path: "/works/{id}/withdraw",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "add_unit",
        method: "POST",
        path: "/works/{id}/chapters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "reorder_units",
        method: "POST",
        path: "/works/{id}/reorder-units",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "update_unit",
        method: "PATCH",
        path: "/chapters/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "delete_unit",
        method: "DELETE",
        path: "/chapters/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "list_revisions",
        method: "GET",
        path: "/chapters/{id}/revisions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "restore_revision",
        method: "POST",
        path: "/chapters/{id}/restore-revision",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "read_work",
        method: "GET",
        path: "/works/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "works.rs",
        handler: "read_unit",
        method: "GET",
        path: "/works/{id}/chapters/{unit}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "works.rs",
        handler: "toggle_kudos",
        method: "POST",
        path: "/works/{id}/kudos",
        audience: Audience::Scoped,
    },
    // §50.1's highlight. `Authenticated`, NOT `Scoped`, and the distinction is the
    // point rather than a detail:
    //
    //   * `Scoped` means `RequireActorScoped`, which exists for §23.2's bot actions
    //     -- a token that carries an explicit scope set and an acting pseud. It
    //     answers a token lacking the scope with 403 and never reveals the resource.
    //   * a highlight is a signed-in reader's own note about a span. There is no
    //     token scope that ought to authorise it, and giving one would let a bot
    //     fabricate reader highlights -- which is exactly the influence-purchase
    //     §50.5 rules out.
    //
    // So the handler takes `RequireSession`, then runs `reading_decision` to decide
    // this reader's access to THIS work, and a highlight counts once toward the
    // work: a stranger cannot buy gravity with quote volume (§50.3).
    RouteEntry {
        file: "works.rs",
        handler: "create_highlight",
        method: "POST",
        path: "/works/{id}/highlights",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Reading progress, ratings, reviews, history, notes — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "reading.rs",
        handler: "save_progress",
        method: "PUT",
        path: "/reading/progress",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "get_progress",
        method: "GET",
        path: "/reading/progress",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "forget_progress",
        method: "DELETE",
        path: "/reading/progress",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "list_history",
        method: "GET",
        path: "/library/history",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "clear_history",
        method: "POST",
        path: "/library/history/clear",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "delete_history",
        method: "DELETE",
        path: "/library/history/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "upsert_dnf",
        method: "POST",
        path: "/works/{id}/dnf",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "upsert_dnf",
        method: "PUT",
        path: "/works/{id}/dnf",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "delete_dnf",
        method: "DELETE",
        path: "/works/{id}/dnf",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "get_dnf",
        method: "GET",
        path: "/works/{id}/dnf",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "get_dnf_reasons",
        method: "GET",
        path: "/works/{id}/dnf/reasons",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "dnf.rs",
        handler: "list_my_dnf",
        method: "GET",
        path: "/me/dnf",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "get_rating",
        method: "GET",
        path: "/works/{id}/rating",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "upsert_rating",
        method: "PUT",
        path: "/works/{id}/rating",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "delete_rating",
        method: "DELETE",
        path: "/works/{id}/rating",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "list_reviews",
        method: "GET",
        path: "/works/{id}/reviews",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "upsert_review",
        method: "PUT",
        path: "/works/{id}/reviews",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "delete_review",
        method: "DELETE",
        path: "/works/{id}/reviews",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "list_notes",
        method: "GET",
        path: "/notes",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "upsert_note",
        method: "PUT",
        path: "/notes",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "reading.rs",
        handler: "delete_note",
        method: "DELETE",
        path: "/notes/{id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "rating_integrity.rs",
        handler: "get_rating_summary",
        method: "GET",
        path: "/works/{id}/rating-summary",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "rating_integrity.rs",
        handler: "list_anomalies",
        method: "GET",
        path: "/admin/anomalies",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "rating_integrity.rs",
        handler: "clear_anomaly",
        method: "POST",
        path: "/admin/anomalies/{id}/clear",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "works.rs",
        handler: "list_lineage",
        method: "GET",
        path: "/works/{id}/lineage",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "works.rs",
        handler: "add_lineage",
        method: "POST",
        path: "/works/{id}/lineage",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "decision_service.rs",
        handler: "evaluate_decision",
        method: "POST",
        path: "/decisions/evaluate",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "decision_service.rs",
        handler: "list_tasks",
        method: "GET",
        path: "/decisions/tasks",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "decision_service.rs",
        handler: "get_audit_trail",
        method: "GET",
        path: "/decisions/audit",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "decision_service.rs",
        handler: "trigger_backfill",
        method: "POST",
        path: "/decisions/backfill/{task}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "account_permissions.rs",
        handler: "get_account_permissions",
        method: "GET",
        path: "/me/permissions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "account_permissions.rs",
        handler: "put_account_permissions",
        method: "PUT",
        path: "/me/permissions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "explain_slot",
        method: "GET",
        path: "/discovery/slots/{slot_id}/explanation",
        // Was tabled as Public when the handler was a stub. It is a reader's
        // own explanation of a slot they were served, so it needs a session --
        // and the handler is scoped to the reader's pseud, so an anonymous
        // caller has no row to be told about.
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "get_attention_report",
        method: "GET",
        path: "/me/attention-report",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Library — session-scoped (all reads and writes)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "library.rs",
        handler: "list_shelves",
        method: "GET",
        path: "/shelves",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "create_shelf",
        method: "POST",
        path: "/shelves",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "get_shelf",
        method: "GET",
        path: "/shelves/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "patch_shelf",
        method: "PATCH",
        path: "/shelves/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "delete_shelf_route",
        method: "DELETE",
        path: "/shelves/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "add_to_shelf",
        method: "POST",
        path: "/shelves/{id}/items/{item_id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "remove_from_shelf",
        method: "DELETE",
        path: "/shelves/{id}/items/{item_id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "list_bookmarks",
        method: "GET",
        path: "/bookmarks",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "create_bookmark",
        method: "POST",
        path: "/bookmarks",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "library.rs",
        handler: "get_bookmark",
        method: "GET",
        path: "/bookmarks/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "patch_bookmark",
        method: "PATCH",
        path: "/bookmarks/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "delete_bookmark_route",
        method: "DELETE",
        path: "/bookmarks/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "query_library_items",
        method: "GET",
        path: "/library/items",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "list_item_tags",
        method: "GET",
        path: "/library/items/{id}/tags",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "add_item_tag",
        method: "PUT",
        path: "/library/items/{id}/tags/{tag}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "remove_item_tag",
        method: "DELETE",
        path: "/library/items/{id}/tags/{tag}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "read_status",
        method: "GET",
        path: "/library/items/{id}/status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "set_status",
        method: "PUT",
        path: "/library/items/{id}/status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "clear_status",
        method: "DELETE",
        path: "/library/items/{id}/status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "list_views",
        method: "GET",
        path: "/saved-views",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "create_view",
        method: "POST",
        path: "/saved-views",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "get_view",
        method: "GET",
        path: "/saved-views/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "patch_view",
        method: "PATCH",
        path: "/saved-views/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "delete_view_route",
        method: "DELETE",
        path: "/saved-views/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "start_update_check",
        method: "POST",
        path: "/library/updates/check",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "storage_usage",
        method: "GET",
        path: "/library/storage",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "library.rs",
        handler: "batch_remove_items",
        method: "POST",
        path: "/library/items/batch",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Jobs — session-scoped (caller's own queue)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "jobs.rs",
        handler: "list_jobs",
        method: "GET",
        path: "/jobs",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "jobs.rs",
        handler: "start_job",
        method: "POST",
        path: "/jobs",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "jobs.rs",
        handler: "cancel_job",
        method: "POST",
        path: "/jobs/{id}/cancel",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "jobs.rs",
        handler: "list_all_jobs",
        method: "GET",
        path: "/admin/jobs",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "jobs.rs",
        handler: "retry_job",
        method: "POST",
        path: "/admin/jobs/{id}/retry",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Imports — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "imports.rs",
        handler: "list_sources",
        method: "GET",
        path: "/imports/sources",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "preview_import",
        method: "POST",
        path: "/imports/preview",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "list_imports",
        method: "GET",
        path: "/imports",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "start_import",
        method: "POST",
        path: "/imports",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "get_import",
        method: "GET",
        path: "/imports/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "cancel_import",
        method: "POST",
        path: "/imports/{id}/cancel",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "import_shelf_csv",
        method: "POST",
        path: "/library/imports/csv",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "retry_failed_units",
        method: "POST",
        path: "/imports/{id}/retry-failed-units",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "list_credentials",
        method: "GET",
        path: "/source-credentials",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "store_credential",
        method: "POST",
        path: "/source-credentials",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "delete_credential",
        method: "DELETE",
        path: "/source-credentials/{id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "test_credential",
        method: "POST",
        path: "/source-credentials/{id}/test",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Imports — admin surface (operator-gated, spec §11.8)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "imports.rs",
        handler: "revision_cache_stats",
        method: "GET",
        path: "/admin/sources/revisions",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "clear_revision_cache",
        method: "DELETE",
        path: "/admin/sources/revisions",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "purge_revision_cache",
        method: "POST",
        path: "/admin/sources/revisions/purge",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "sweep_source_health",
        method: "POST",
        path: "/admin/sources/health",
        audience: Audience::Operator,
    },
    // ------------------------------------------------------------------
    // Retention — operator-gated (spec §11.15, amendment §4.1-4.2)
    //
    // The amendment says `works_past_saving` "changes no behaviour and it is
    // not public", so it is Operator like the rest of this block rather than
    // Public or Stats — and the test for that is in retention_routes.rs, which
    // asserts a non-operator gets a 404 rather than a 403.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "retention.rs",
        handler: "get_policy",
        method: "GET",
        path: "/admin/retention/policy",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "patch_policy",
        method: "PATCH",
        path: "/admin/retention/policy",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "get_sources",
        method: "GET",
        path: "/admin/retention/sources",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "unsupported_verb",
        method: "POST",
        path: "/admin/retention/sources",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "put_source_override",
        method: "PUT",
        path: "/admin/retention/sources/{source_key}",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "delete_source_override",
        method: "DELETE",
        path: "/admin/retention/sources/{source_key}",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention.rs",
        handler: "get_works_past_saving",
        method: "GET",
        path: "/admin/retention/works-past-saving",
        audience: Audience::Operator,
    },
    // ------------------------------------------------------------------
    // Retention proposals (plan E, spec §5 / §19.15). The reader half is
    // `RouteClass::Write` at the mount but `Audience::Authenticated`, because
    // the door is `RequireSession` and the gate beyond it is
    // `retention_governance.proposal_min_trust` — a trust level a reader can
    // earn. `require_operator` would be the wrong extractor for the person the
    // feature is for, and recording the audience as `Operator` here would make
    // the inventory describe a gate the code does not have. There is no
    // `Audience::Reader`: the enum names the DOOR, and the trust gate is a
    // second thing the inventory does not model. `roadmap.rs`'s
    // `post_suggest` is the same shape, for the same reason.
    // ------------------------------------------------------------------
    // §11.15b: a reader's own copy of an external body. Both are
    // `Authenticated` and not `Public`, because the copy is the requesting
    // reader's and the trust bar is checked on the POST — a GET that served
    // bytes to an unauthenticated caller would hand every copy to every reader.
    RouteEntry {
        file: "reader_body_copies.rs",
        handler: "request",
        method: "POST",
        path: "/works/{id}/body-request",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reader_body_copies.rs",
        handler: "status",
        method: "GET",
        path: "/works/{id}/body-request",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "retention_proposals.rs",
        handler: "list",
        method: "GET",
        path: "/retention/proposals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "retention_proposals.rs",
        handler: "create",
        method: "POST",
        path: "/retention/proposals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "retention_proposals.rs",
        handler: "one",
        method: "GET",
        path: "/retention/proposals/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "retention_proposals.rs",
        handler: "vote",
        method: "POST",
        path: "/retention/proposals/{id}/vote",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "retention_proposal_admin.rs",
        handler: "respond",
        method: "POST",
        path: "/admin/retention/proposals/{id}/respond",
        audience: Audience::Operator,
    },
    RouteEntry {
        // The handler is `override_setting` because `override` is a reserved
        // word in every Rust edition; the PATH is `/override`, which is the
        // contract a client sees. Reading `handler: "override"` off the route
        // string is the mistake this row's value exists to prevent.
        file: "retention_proposal_admin.rs",
        handler: "override_setting",
        method: "POST",
        path: "/admin/retention/proposals/{id}/override",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "retention_settle.rs",
        handler: "settle_now",
        method: "POST",
        path: "/admin/retention/proposals/settle",
        audience: Audience::Operator,
    },
    // ------------------------------------------------------------------
    // Exports — session-scoped (authed_router)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "exports.rs",
        handler: "download_by_token",
        method: "GET",
        path: "/exports/download/{token}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "list_formats",
        method: "GET",
        path: "/exports/formats",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "list_exports",
        method: "GET",
        path: "/exports",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "start_export",
        method: "POST",
        path: "/exports",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "get_export",
        method: "GET",
        path: "/exports/{id}",
        audience: Audience::Scoped,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "mint_grant",
        method: "POST",
        path: "/exports/{id}/grant",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "download_own",
        method: "GET",
        path: "/exports/{id}/download",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "start_bulk_export",
        method: "POST",
        path: "/exports/bulk",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "forget_export",
        method: "DELETE",
        path: "/exports/{id}",
        audience: Audience::Scoped,
    },
    // ------------------------------------------------------------------
    // Feedback — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "feedback.rs",
        handler: "get_preferences",
        method: "GET",
        path: "/feedback/preferences",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "put_preferences",
        method: "PUT",
        path: "/feedback/preferences",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "get_work_policy",
        method: "GET",
        path: "/feedback/preferences/works/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "put_work_policy",
        method: "PUT",
        path: "/feedback/preferences/works/{id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "get_inbox",
        method: "GET",
        path: "/feedback/inbox",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "allow_pseud",
        method: "POST",
        path: "/feedback/allow/{pseud}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "feedback.rs",
        handler: "deny_pseud",
        method: "POST",
        path: "/feedback/deny/{pseud}",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Search — public reads
    // ------------------------------------------------------------------
    RouteEntry {
        file: "search.rs",
        handler: "search",
        method: "GET",
        path: "/search",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "search.rs",
        handler: "in_work",
        method: "GET",
        path: "/search/in-work/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "search.rs",
        handler: "users_search",
        method: "GET",
        path: "/users/search",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Discovery — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "discovery.rs",
        handler: "get_discovery",
        method: "GET",
        path: "/discovery",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_taste_profile",
        method: "GET",
        path: "/discovery/taste-profile",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "recompute_taste_profile",
        method: "POST",
        path: "/discovery/taste-profile/recompute",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "clear_taste_profile",
        method: "POST",
        path: "/discovery/taste-profile/clear",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_blind_date",
        method: "GET",
        path: "/discovery/blind-date",
        audience: Audience::Authenticated,
    },
    // Item 7, spec §16.10: one work from outside the reader's taste profile.
    //
    // Authenticated rather than public, and not by accident: the query excludes work sharing
    // a tag with the reader's profile and reads their bookmarks, so the answer is a function
    // of who is asking. A public call would return a pick shaped by nobody's taste, which is
    // the same set as Blind Date's and would make the two surfaces indistinguishable.
    RouteEntry {
        file: "discovery.rs",
        handler: "get_surprise_me",
        method: "GET",
        path: "/discovery/surprise-me",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "continue_reading.rs",
        handler: "continue_reading",
        method: "GET",
        path: "/continue-reading",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // M45-51 — subject access and erasure. Both are reader-scoped and take no
    // path parameter, so the path cannot be guessed at from the URL and both
    // are POST/GET on `/me/...` rather than on a resource of their own.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "erasure.rs",
        handler: "get_data",
        method: "GET",
        path: "/me/data",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "erasure.rs",
        handler: "post_erasure",
        method: "POST",
        path: "/me/erasure",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Pre-read reports — author-only. §23.7: a report is provider-specific and
    // the author is the only person who may see which provider produced it, or
    // withdraw consent from one without discarding another's output.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "preread.rs",
        handler: "get_preread",
        method: "GET",
        path: "/works/{work_id}/preread",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "preread.rs",
        handler: "get_preread_providers",
        method: "GET",
        path: "/works/{work_id}/preread/providers",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "preread.rs",
        handler: "forget_preread_provider",
        method: "DELETE",
        path: "/works/{work_id}/preread/{provider}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "set_operator_affinity",
        method: "POST",
        path: "/operator/affinities",
        audience: Audience::Authenticated,
    },
    // M52-08: the shadow evaluation is instance-tuning diagnostics about the
    // ranker, so it is an operator door like the settings it reports on.
    RouteEntry {
        file: "discovery.rs",
        handler: "get_shadow_evaluation",
        method: "GET",
        path: "/operator/rec/shadow",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Browse — sort vocabulary surfaces (spec §43.1)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "browse.rs",
        handler: "list_surfaces",
        method: "GET",
        path: "/browse/surfaces",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "get_sort",
        method: "GET",
        path: "/browse/sort/{surface}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "set_sort",
        method: "PUT",
        path: "/browse/sort/{surface}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "delete_sort",
        method: "DELETE",
        path: "/browse/sort/{surface}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "list_people",
        method: "GET",
        path: "/people",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "list_tags",
        method: "GET",
        path: "/tags",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "list_works_by_tag",
        method: "GET",
        path: "/tags/{tag}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "list_fandoms",
        method: "GET",
        path: "/fandoms",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "browse.rs",
        handler: "list_works_by_fandom",
        method: "GET",
        path: "/fandoms/{fandom}",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Quiz — onboarding taste quiz (spec §0.4.2)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "quiz.rs",
        handler: "get_quiz_works",
        method: "GET",
        path: "/quiz/works",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "quiz.rs",
        handler: "post_quiz_answers",
        method: "POST",
        path: "/quiz/answers",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "quiz.rs",
        handler: "get_my_quiz_answers",
        method: "GET",
        path: "/quiz/answers",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "quiz.rs",
        handler: "skip_quiz",
        method: "POST",
        path: "/quiz/skip",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "quiz.rs",
        handler: "admin_list_quiz_works",
        method: "GET",
        path: "/operator/quiz-works",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Tasting menu — the uncertainty calibration queue (spec §49.5, M45-19)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "tasting.rs",
        handler: "get_queue",
        method: "GET",
        path: "/tasting/queue",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "tasting.rs",
        handler: "post_response",
        method: "POST",
        path: "/tasting/respond",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "tasting.rs",
        handler: "get_my_responses",
        method: "GET",
        path: "/tasting/responses",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Discovery — nested sub-routers (recipe_routes, dashboard_routes)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "discovery.rs",
        handler: "create_recipe",
        method: "POST",
        path: "/recipes/",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_recipe_route",
        method: "GET",
        path: "/recipes/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "update_recipe_route",
        // POST, not PATCH. The route is registered
        // `.route("/{id}", get(get_recipe_route).post(update_recipe_route))`, and
        // this row said PATCH — which nothing checked, because the direction test
        // compared file, path and handler and left the method out of the
        // comparison entirely. The method is now compared, so the two halves of
        // ROUTE_TABLE can no longer disagree with the router about what a route is.
        method: "POST",
        path: "/recipes/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "delete_recipe_route",
        method: "POST",
        path: "/recipes/{id}/delete",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "list_recipes_route",
        method: "GET",
        path: "/recipes/list",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_dashboard",
        method: "GET",
        path: "/dashboard/",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "save_dashboard",
        method: "POST",
        path: "/dashboard/",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Community — mixed (public reads, session-scoped writes)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "community.rs",
        handler: "get_work_comments",
        method: "GET",
        path: "/works/{id}/comments",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_comment",
        method: "POST",
        path: "/works/{id}/comments",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "delete_comment",
        method: "POST",
        path: "/comments/{id}/delete",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_forums",
        method: "GET",
        path: "/forums",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_forums_topics",
        method: "GET",
        path: "/forums/{category}/topics",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_topic",
        method: "POST",
        path: "/forums/{category}/topics",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_topic",
        method: "GET",
        path: "/topics/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_topic_replies",
        method: "GET",
        path: "/topics/{id}/replies",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_reply",
        method: "POST",
        path: "/topics/{id}/replies",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "lock_topic",
        method: "POST",
        path: "/topics/{id}/lock",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_groups",
        method: "GET",
        path: "/groups",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_group",
        method: "POST",
        path: "/groups",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_group",
        method: "GET",
        path: "/groups/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "join_group",
        method: "POST",
        path: "/groups/{id}/join",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "leave_group",
        method: "POST",
        path: "/groups/{id}/leave",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "put_role",
        method: "PUT",
        path: "/groups/{id}/role",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_conversations",
        method: "GET",
        path: "/conversations",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_conversation",
        method: "POST",
        path: "/conversations",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_conv_messages",
        method: "GET",
        path: "/conversations/{id}/messages",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_message",
        method: "POST",
        path: "/conversations/{id}/messages",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_blocks",
        method: "GET",
        path: "/me/blocks",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_block",
        method: "POST",
        path: "/me/blocks",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "delete_block",
        method: "DELETE",
        path: "/me/blocks/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_mutes",
        method: "GET",
        path: "/me/mutes",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "post_mute",
        method: "POST",
        path: "/me/mutes",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "delete_mute",
        method: "DELETE",
        path: "/me/mutes/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_presence_stream",
        method: "GET",
        path: "/presence/stream",
        audience: Audience::Authenticated,
    },
    // Presence being opt-in is only true if a person can decline it, so the
    // flag has to be settable by its owner.
    RouteEntry {
        file: "community.rs",
        handler: "set_presence_preference",
        method: "PUT",
        path: "/me/presence",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Taxonomy — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "taxonomy.rs",
        handler: "autocomplete",
        method: "GET",
        path: "/taxonomy",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "taxonomy.rs",
        handler: "create_taxonomy_node",
        method: "POST",
        path: "/taxonomy",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "taxonomy.rs",
        handler: "get_node",
        method: "GET",
        path: "/taxonomy/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "taxonomy.rs",
        handler: "create_taxonomy_alias",
        method: "POST",
        path: "/taxonomy/aliases",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "taxonomy.rs",
        handler: "tag_work",
        method: "POST",
        path: "/works/{id}/tags",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Events — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "events.rs",
        handler: "get_collections",
        method: "GET",
        path: "/collections",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_collection",
        method: "POST",
        path: "/collections",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_collection",
        method: "GET",
        path: "/collections/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "put_collection",
        method: "PUT",
        path: "/collections/{id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_collection_items",
        method: "GET",
        path: "/collections/{id}/items",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_collection_item",
        method: "POST",
        path: "/collections/{id}/items",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_challenges",
        method: "GET",
        path: "/challenges",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_challenge",
        method: "POST",
        path: "/challenges",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_challenge",
        method: "GET",
        path: "/challenges/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_challenge_entry",
        method: "POST",
        path: "/challenges/{id}/entries",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_requests",
        method: "GET",
        path: "/requests",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_request",
        method: "POST",
        path: "/requests",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_claim",
        method: "POST",
        path: "/requests/{id}/claims",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_fulfil_claim",
        method: "POST",
        path: "/claims/{id}/fulfil",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_wishlist",
        method: "GET",
        path: "/wishlists/{account}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_wishlist_item",
        method: "POST",
        path: "/wishlist-items",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_events",
        method: "GET",
        path: "/events",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "post_event",
        method: "POST",
        path: "/events",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "events.rs",
        handler: "get_event",
        method: "GET",
        path: "/events/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "events.rs",
        handler: "join_event",
        method: "POST",
        path: "/events/{id}/join",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Governance — mixed (public reports, session-scoped moderation)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "governance.rs",
        handler: "submit_report",
        method: "POST",
        path: "/reports",
        audience: Audience::Public,
    },
    // The modlog: readable by any signed-in reader (spec §14.6). **Not
    // `Public`** — an anonymous reader gets 401, not the record.
    // The "more like this" ladder (spec §11.5). **Public**: a reader with no
    // session may ask what is next to a work.
    RouteEntry {
        file: "works.rs",
        handler: "more_like_this",
        method: "GET",
        path: "/works/{id}/more-like-this",
        audience: Audience::Public,
    },
    // The author's explicit relations (spec §6.2).
    RouteEntry {
        file: "works.rs",
        handler: "relate_work",
        method: "POST",
        path: "/works/{id}/relations",
        audience: Audience::Authenticated,
    },
    // The author's series membership (spec §6.2).
    RouteEntry {
        file: "works.rs",
        handler: "put_work_in_series",
        method: "POST",
        path: "/works/{id}/series",
        audience: Audience::Authenticated,
    },
    // The author's own declaration that the work contains generated content
    // (spec §15.4, N-4). **A declaration, never a detection.**
    RouteEntry {
        file: "works.rs",
        handler: "declare_generated_content",
        method: "PATCH",
        path: "/works/{id}/generated-content",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "modlog",
        method: "GET",
        path: "/modlog",
        audience: Audience::Authenticated,
    },
    // An account's trust history: the events, not just the level.
    RouteEntry {
        file: "governance.rs",
        handler: "account_trust_history",
        method: "GET",
        path: "/accounts/{id}/trust",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "list_reports",
        method: "GET",
        path: "/reports",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "get_report",
        method: "GET",
        path: "/reports/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "moderation_queue",
        method: "GET",
        path: "/moderation/queue",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "assign_task",
        method: "POST",
        path: "/moderation/reports/{id}/assign",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "decide_task",
        method: "POST",
        path: "/moderation/tasks/{id}/decide",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "issue_sanction",
        method: "POST",
        path: "/sanctions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "lift_sanction",
        method: "POST",
        path: "/sanctions/{id}/lift",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "my_sanctions",
        method: "GET",
        path: "/me/sanctions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "open_appeal",
        method: "POST",
        path: "/appeals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "decide_appeal",
        method: "POST",
        path: "/appeals/{id}/decide",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "list_my_appeals",
        method: "GET",
        path: "/appeals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "my_trust",
        method: "GET",
        path: "/me/trust",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "governance.rs",
        handler: "my_audit_log",
        method: "GET",
        path: "/me/audit-log",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Economy — mixed (public reads, session-scoped writes)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "economy.rs",
        handler: "get_credits",
        method: "GET",
        path: "/credits",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "get_quote",
        method: "GET",
        path: "/credits/quote",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "post_reserve",
        method: "POST",
        path: "/credits/reserve",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "get_queue_position",
        method: "GET",
        path: "/jobs/{job_id}/queue-position",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "get_usage",
        method: "GET",
        path: "/usage",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "list_bounties",
        method: "GET",
        path: "/bounties",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "create_bounty",
        method: "POST",
        path: "/bounties",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "claim_bounty",
        method: "POST",
        path: "/bounties/{bounty_id}/claim",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "contribute_to_bounty",
        method: "POST",
        path: "/bounties/{bounty_id}/contribute",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "economy.rs",
        handler: "get_subscription",
        method: "GET",
        path: "/subscription",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Marketplace — mixed (public reads, session-scoped writes)
    // ------------------------------------------------------------------
    // Curator-submitted source adapters — M45-57, spec §55.2.
    //
    // `Audience::Authenticated` (RequireSession), not Public: the queue exposes
    // manifests naming sites to crawl and selectors to use. Every one of the
    // four is behind a session; §55.2's TL3 bar is enforced in the store, and a
    // Public audience here would mean the 401 line of §55.8 lives in the
    // handler rather than in the extractor.
    RouteEntry {
        file: "source_adapters.rs",
        handler: "submit_adapter",
        method: "POST",
        path: "/extensions/source-adapters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "source_adapters.rs",
        handler: "list_submissions",
        method: "GET",
        path: "/extensions/source-adapters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "source_adapters.rs",
        handler: "get_submission",
        method: "GET",
        path: "/extensions/source-adapters/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "source_adapters.rs",
        handler: "list_reviews",
        method: "GET",
        path: "/extensions/source-adapters/{id}/reviews",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "source_adapters.rs",
        handler: "record_review",
        method: "POST",
        path: "/extensions/source-adapters/{id}/reviews",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // M45-22 (§54): the personal concierge. Five routes, all behind a session.
    //
    // `render_queue` is Authenticated rather than Public for a reason that is not
    // about secrecy: it records a session (§54.3) and reads the reader's own
    // history to exclude already-read works, so an unauthenticated call has no
    // account to scope any of that to.
    RouteEntry {
        file: "concierge.rs",
        handler: "render_queue",
        method: "GET",
        path: "/me/concierge",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "concierge.rs",
        handler: "list_sessions",
        method: "GET",
        path: "/me/concierge/sessions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "concierge.rs",
        handler: "list_watches",
        method: "GET",
        path: "/me/watches",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "concierge.rs",
        handler: "put_watch",
        method: "PUT",
        path: "/me/watches/{work_id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "concierge.rs",
        handler: "delete_watch",
        method: "DELETE",
        path: "/me/watches/{work_id}",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Rows the route walker found once it was made to actually walk.
    //
    // Every one of these is a route that has been registered and served for
    // however long `discovery.rs` has existed, and none of them was in this table.
    // They went unnoticed because every multi-handler `.route(...)` line only ever
    // had its FIRST method tabulated: `get(a).post(b)` produced a table row for `a`
    // and nothing for `b`, and the direction test compared an empty walk against it.
    //
    // So this is not new debt — it is old debt that a test which collected nothing
    // could not see. The comment on each is not needed; the shape is the lesson.
    RouteEntry {
        file: "cta.rs",
        handler: "list",
        method: "GET",
        path: "/works/{id}/cta_marks",
        // Public. The handler takes `MaybeSession` rather than nothing, so the
        // door is declared; reading the POST side's audience by analogy would be
        // wrong, because that one does record who the reader is.
        audience: Audience::Public,
    },
    RouteEntry {
        file: "cta.rs",
        handler: "retract",
        method: "DELETE",
        path: "/works/{id}/cta_marks/me",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "post_arena_vote",
        method: "POST",
        path: "/roadmap/arena",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "create_list",
        method: "POST",
        path: "/directory/lists",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "submit_entry",
        method: "POST",
        path: "/directory/entries",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "put_progress",
        method: "PUT",
        path: "/works/{id}/progress",
        // RequirePseud, not RequireSession: a reader's spoiler position belongs to
        // the pseud that set it, and switching faces must not carry it across.
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "post_warning",
        method: "POST",
        path: "/posts/{id}/warnings",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "put_mode",
        method: "PUT",
        path: "/topics/{id}/mode",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Rows found by the parenthesis-delimited walk, which sees a `.route(...)`
    // rustfmt wrapped across lines. Every one of these has been registered and
    // served for as long as the module existed. They were invisible because the
    // walk was line-at-a-time: the line carrying `.route(` had no path on it, and
    // the lines carrying the path had no `.route(` on them.
    //
    // So this is not new debt. It is old debt that a walk which collected nothing
    // could not see, and the count is the honest measure of how much of it there
    // was: 19 routes across 9 modules, plus one stale duplicate row that claimed
    // GET for a DELETE route.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "marketplace.rs",
        handler: "delete_webhook",
        method: "DELETE",
        path: "/me/webhooks",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "get_instance_theme",
        method: "GET",
        path: "/federation/theme",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "set_theme_visibility",
        method: "PUT",
        path: "/federation/theme",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "get_work_ai_declaration",
        method: "GET",
        path: "/works/{work_id}/ai-declaration",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "quiz.rs",
        handler: "admin_set_quiz_works",
        method: "POST",
        path: "/operator/quiz-works",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "get_rec_engine",
        method: "GET",
        path: "/settings/recommendations",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "settings.rs",
        handler: "patch_rec_engine",
        method: "PATCH",
        path: "/settings/recommendations",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "remove_entry",
        method: "DELETE",
        path: "/directory/entries/{id}",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "post_draft",
        method: "POST",
        path: "/topics/{id}/draft",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "delete_draft",
        method: "DELETE",
        path: "/topics/{id}/draft",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "put_warning_pref",
        method: "PUT",
        path: "/me/warning-prefs",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "add_schedule_section",
        method: "POST",
        path: "/topics/{id}/schedule",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "post_wiki_pin",
        method: "POST",
        path: "/topics/{id}/wiki-pin",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "approve_wiki_pin",
        method: "PUT",
        path: "/topics/{id}/wiki-pin",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "works.rs",
        handler: "read_work_status",
        method: "GET",
        path: "/works/{id}/reading-status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "set_work_status",
        method: "PUT",
        path: "/works/{id}/reading-status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "clear_work_status",
        method: "DELETE",
        path: "/works/{id}/reading-status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "get_work_permissions",
        method: "GET",
        path: "/works/{id}/permissions",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "works.rs",
        handler: "put_work_permissions",
        method: "PUT",
        path: "/works/{id}/permissions",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    RouteEntry {
        file: "marketplace.rs",
        handler: "list_listings",
        method: "GET",
        path: "/listings",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "create_listing",
        method: "POST",
        path: "/listings",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "create_commission",
        method: "POST",
        path: "/listings/{id}/commissions",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "transition_commission",
        method: "POST",
        path: "/commissions/{id}/transition",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "list_extensions",
        method: "GET",
        path: "/extensions",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "get_extension",
        method: "GET",
        path: "/extensions/{slug}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "grant_extension",
        method: "POST",
        path: "/extensions/{slug}/grant",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "revoke_extension",
        method: "POST",
        path: "/extensions/{slug}/revoke",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "list_my_grants",
        method: "GET",
        path: "/me/extension-grants",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "list_webhooks",
        method: "GET",
        path: "/me/webhooks",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "create_webhook",
        method: "POST",
        path: "/me/webhooks",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "list_gallery",
        method: "GET",
        path: "/works/{id}/gallery",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "marketplace.rs",
        handler: "add_gallery_item",
        method: "POST",
        path: "/works/{id}/gallery",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Translation — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "translation.rs",
        handler: "create_translation",
        method: "POST",
        path: "/works/{id}/translations",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "get_translation",
        method: "GET",
        path: "/translations/{job_id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "transition_translation",
        method: "POST",
        path: "/translations/{job_id}/transition",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "list_memory",
        method: "GET",
        path: "/me/translation-memory",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "add_glossary_term",
        method: "POST",
        path: "/me/translation-glossaries",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "open_review",
        method: "POST",
        path: "/translations/{job_id}/reviews",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "translation.rs",
        handler: "decide_review",
        method: "POST",
        path: "/translation-reviews/{review_id}/decide",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // External — public reads, mixed auth
    // ------------------------------------------------------------------
    RouteEntry {
        file: "external.rs",
        handler: "get_public_work",
        method: "GET",
        path: "/public/works/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "public_search",
        method: "GET",
        path: "/public/search",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "list_tokens",
        method: "GET",
        path: "/me/tokens",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "issue_token",
        method: "POST",
        path: "/me/tokens",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "revoke_token",
        method: "POST",
        path: "/me/tokens/{id}",
        audience: Audience::Public,
    },
    // `Audience::Actor`, not `Scoped` and not `Public`.
    //
    // Not `Public`: the route requires a credential; anonymous callers get 401.
    //
    // Not `Scoped`: `RequireActorScoped` would answer a token with no scopes
    // with 403 and never reveal the credential exists, which defeats the
    // endpoint's purpose. A bot whose reader revoked every scope has to be able
    // to learn that, and "you are a token with no scopes" is the answer.
    //
    // The audience is checked against the handler's own extractor, so this is a
    // claim about what the route discloses rather than a label.
    RouteEntry {
        file: "external.rs",
        handler: "describe_credential",
        method: "GET",
        path: "/me/credential",
        audience: Audience::Actor,
    },
    RouteEntry {
        file: "external.rs",
        handler: "register_bot",
        method: "POST",
        path: "/me/bots",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "get_rss_feed",
        method: "GET",
        path: "/feeds/{handle}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "get_atom_feed",
        method: "GET",
        path: "/feeds/{handle}/atom",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "subscribe_push",
        method: "POST",
        path: "/me/push/subscribe",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "openapi.rs",
        handler: "get_openapi_spec",
        method: "GET",
        path: "/openapi.json",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "external.rs",
        handler: "get_ai_work",
        method: "GET",
        path: "/ai/works/{id}",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Admin — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "admin.rs",
        handler: "get_stats",
        method: "GET",
        path: "/admin/stats",
        audience: Audience::Public,
    },
    // §52.1's leakage view. `Operator` because the handler takes `RequireSession`
    // and then `require_operator` -- and that second check is the load-bearing
    // part: it answers 404 rather than 403 to a non-operator, since for this view
    // the endpoint's EXISTENCE is a disclosure (a reader probing
    // /admin/discovery learns the operator runs a leakage review, which is one of
    // the things the review is about).
    RouteEntry {
        file: "admin_discovery.rs",
        handler: "get_leakage",
        method: "GET",
        path: "/admin/discovery/leakage",
        audience: Audience::Operator,
    },
    // §53's faucet/sink view. `Operator`, like the leakage view beside it, and for the
    // same reason: the route itself is `RequireSession` and calls `require_operator`
    // inside, so a non-operator gets 404 rather than a 403 that would confirm the
    // dashboard exists.
    RouteEntry {
        file: "flows.rs",
        handler: "get_flows",
        method: "GET",
        path: "/admin/economy/flows",
        audience: Audience::Operator,
    },
    // Same shape and the same reason: `RequireSession` plus an internal
    // `require_operator`, so a non-operator gets 404 rather than a 403 that would confirm
    // this instance measures its own discovery quality.
    RouteEntry {
        file: "north_star.rs",
        handler: "get_north_star",
        method: "GET",
        path: "/admin/metrics/north-star",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "admin.rs",
        handler: "record_admin_action",
        method: "POST",
        path: "/admin/actions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "admin.rs",
        handler: "list_privacy_requests",
        method: "GET",
        path: "/me/privacy-requests",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "admin.rs",
        handler: "create_privacy_request",
        method: "POST",
        path: "/me/privacy-requests",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "admin.rs",
        handler: "check_abuse_status",
        method: "GET",
        path: "/admin/abuse-status/{key}",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Collaborators — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "collaborators.rs",
        handler: "create_invitation",
        method: "POST",
        path: "/works/{id}/contributors/invitations",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "update_contributor",
        method: "PATCH",
        path: "/works/{id}/contributors/{pseud}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "remove_contributor",
        method: "DELETE",
        path: "/works/{id}/contributors/{pseud}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "list_invitations",
        method: "GET",
        path: "/invitations",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "accept_invitation",
        method: "POST",
        path: "/invitations/{id}/accept",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "decline_invitation",
        method: "POST",
        path: "/invitations/{id}/decline",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "collaborators.rs",
        handler: "revoke_invitation",
        method: "POST",
        path: "/invitations/{id}/revoke",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Media — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "media.rs",
        handler: "list_media",
        method: "GET",
        path: "/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "media_feed",
        method: "GET",
        path: "/media/feed",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "get_media",
        method: "GET",
        path: "/media/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_media_files",
        method: "GET",
        path: "/media/{id}/files",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_media_editions",
        method: "GET",
        path: "/media/{id}/editions",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_creators",
        method: "GET",
        path: "/creators",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "get_creator",
        method: "GET",
        path: "/creators/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "creator_media",
        method: "GET",
        path: "/creators/{id}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_distributors",
        method: "GET",
        path: "/distributors",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "get_distributor",
        method: "GET",
        path: "/distributors/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "distributor_media",
        method: "GET",
        path: "/distributors/{id}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_media_collections",
        method: "GET",
        path: "/media-collections",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "get_media_collection",
        method: "GET",
        path: "/media-collections/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "media_collection_media",
        method: "GET",
        path: "/media-collections/{id}/media",
        audience: Audience::Public,
    },
    // The three doors the direction test found untabled: the collection feed is
    // mounted twice and the kind filter once, and none had a row.
    RouteEntry {
        file: "media.rs",
        handler: "media_collection_feed",
        method: "GET",
        path: "/media-collections/{id}/media/feed",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "list_media_collections_by_kind",
        method: "GET",
        path: "/media-collections/kind/{kind}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "media_collection_feed",
        method: "GET",
        path: "/media-collections/kind/{kind}/media/feed",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "canon_media",
        method: "GET",
        path: "/canons/{id}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "space_media",
        method: "GET",
        path: "/spaces/{id}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "post_media_query",
        method: "POST",
        path: "/media/query",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media.rs",
        handler: "post_creator",
        method: "POST",
        path: "/creators",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media.rs",
        handler: "patch_creator",
        method: "PATCH",
        path: "/creators/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media.rs",
        handler: "post_distributor",
        method: "POST",
        path: "/distributors",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media.rs",
        handler: "post_media_collection",
        method: "POST",
        path: "/media-collections",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media.rs",
        handler: "put_media_collection",
        method: "PUT",
        path: "/media-collections/{id}",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Monetization — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "monetization.rs",
        handler: "set_pricing",
        method: "POST",
        path: "/works/{work_id}/pricing",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "delete_pricing",
        method: "DELETE",
        path: "/works/{work_id}/pricing",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "purchase",
        method: "POST",
        path: "/works/{work_id}/purchase",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "tip",
        method: "POST",
        path: "/works/{work_id}/tips",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "request_payout",
        method: "POST",
        path: "/me/payouts",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "admin_monetization",
        method: "GET",
        path: "/admin/monetization",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "public_pricing",
        method: "GET",
        path: "/works/{work_id}/pricing",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "my_entitlements",
        method: "GET",
        path: "/me/entitlements",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "my_earnings",
        method: "GET",
        path: "/me/earnings",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "create_gift",
        method: "POST",
        path: "/works/{work_id}/gifts",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "list_gifts",
        method: "GET",
        path: "/me/gifts",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Subscriptions — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "subscriptions.rs",
        handler: "subscribe",
        method: "POST",
        path: "/subscriptions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "my_subscriptions",
        method: "GET",
        path: "/subscriptions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "update_subscription",
        method: "PUT",
        path: "/subscriptions/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "unsubscribe",
        method: "DELETE",
        path: "/subscriptions/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "create_alert",
        method: "POST",
        path: "/search-alerts",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "update_alert",
        method: "PUT",
        path: "/search-alerts/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "delete_alert",
        method: "DELETE",
        path: "/search-alerts/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "subscriptions.rs",
        handler: "set_ai_training",
        method: "PUT",
        path: "/works/{work_id}/ai-training",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Narration — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "narration.rs",
        handler: "list_editions",
        method: "GET",
        path: "/works/{id}/editions",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "narration.rs",
        handler: "request_narration",
        method: "POST",
        path: "/works/{id}/editions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "narration.rs",
        handler: "get_edition",
        method: "GET",
        path: "/editions/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "narration.rs",
        handler: "approve_edition",
        method: "POST",
        path: "/editions/{id}/approve",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "narration.rs",
        handler: "download_audio",
        method: "GET",
        path: "/editions/{id}/audio",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Dashboard — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "dashboard.rs",
        handler: "creator_dashboard",
        method: "GET",
        path: "/me/dashboard",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Analytics — trust-gated. Authorisation is in the registry, not here.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "analytics.rs",
        handler: "my_analytics",
        method: "GET",
        path: "/me/analytics",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "analytics.rs",
        handler: "one_capability",
        method: "GET",
        path: "/me/analytics/{capability}",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Lending — session-scoped (except get_lending_status)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "lending.rs",
        handler: "get_lending_status",
        method: "GET",
        path: "/media/{id}/lending",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "lending.rs",
        handler: "lend_work",
        method: "POST",
        path: "/media/{id}/lend",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "lending.rs",
        handler: "revoke_loan",
        method: "PUT",
        path: "/media/{id}/lend",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "lending.rs",
        handler: "list_my_loans",
        method: "GET",
        path: "/me/loans",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Derivatives — public reads, session-scoped writes
    // ------------------------------------------------------------------
    RouteEntry {
        file: "derivative.rs",
        handler: "list_derivatives",
        method: "GET",
        path: "/works/{id}/derivatives",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "derivative.rs",
        handler: "request_derivative",
        method: "POST",
        path: "/works/{id}/derivatives",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "derivative.rs",
        handler: "get_derivative",
        method: "GET",
        path: "/derivatives/{id}",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Notifications — session-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "notifications.rs",
        handler: "list_notifications",
        method: "GET",
        path: "/notifications",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "notifications.rs",
        handler: "read_all",
        method: "POST",
        path: "/notifications/read-all",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "notifications.rs",
        handler: "mark_read",
        method: "POST",
        path: "/notifications/{id}/read",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Work discussion (spec §35.1, M31) — session-scoped, writes pseud-scoped
    // ------------------------------------------------------------------
    RouteEntry {
        file: "work_discussion.rs",
        handler: "get_discussion_mode",
        method: "GET",
        path: "/works/{id}/discussion-mode",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "put_discussion_mode",
        method: "PUT",
        path: "/works/{id}/discussion-mode",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "get_reactions",
        method: "GET",
        path: "/works/{id}/reactions",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "post_reaction",
        method: "POST",
        path: "/works/{id}/reactions",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "get_thread",
        method: "GET",
        path: "/works/{id}/thread",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "post_migrate_comments",
        method: "POST",
        path: "/works/{id}/migrate-comments",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "work_discussion.rs",
        handler: "get_linked_work",
        method: "GET",
        path: "/topics/{id}/work",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Typed votes, budgets, meta-moderation, karma (spec §35.2, M32)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "typed_votes.rs",
        handler: "cast_vote",
        method: "POST",
        path: "/forum/posts/{id}/vote",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "retract_vote",
        method: "DELETE",
        path: "/forum/posts/{id}/vote",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "get_post_votes",
        method: "GET",
        path: "/forum/posts/{id}/votes",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "put_vote_visibility",
        method: "PUT",
        path: "/forum/posts/{id}/vote-visibility",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "post_meta_vote",
        method: "POST",
        path: "/forum/votes/{id}/meta",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "get_category_vote_types",
        method: "GET",
        path: "/forum/categories/{id}/vote-types",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "get_vote_budget",
        method: "GET",
        path: "/me/vote-budget",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "get_own_karma",
        method: "GET",
        path: "/forum/karma",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "typed_votes.rs",
        handler: "get_karma",
        method: "GET",
        path: "/forum/karma/{pseud}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "ap_inbox",
        method: "POST",
        path: "/federation/inbox",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "ap_actor",
        method: "GET",
        path: "/federation/actor",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "ap_outbox",
        method: "GET",
        path: "/federation/outbox",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "recompute_theme",
        method: "POST",
        path: "/federation/theme/recompute",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "list_public_themes",
        method: "GET",
        path: "/federation/themes",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "list_peers",
        method: "GET",
        path: "/federation/peers",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "list_similar_peers",
        method: "GET",
        path: "/federation/peers/similar",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "federation.rs",
        handler: "set_peer_state",
        method: "POST",
        path: "/federation/peers",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "post_sanction",
        method: "POST",
        path: "/mod/sanctions",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "get_sanction_check",
        method: "GET",
        path: "/mod/sanctions/check",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "put_slow_mode",
        method: "PUT",
        path: "/topics/{id}/slow-mode",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "put_federation_scope",
        method: "PUT",
        path: "/topics/{id}/federation-scope",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "post_feature",
        method: "POST",
        path: "/posts/{id}/feature",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "moderation.rs",
        handler: "get_health",
        method: "GET",
        path: "/forum/health",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "get_mode",
        method: "GET",
        path: "/topics/{id}/mode",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "get_schedule",
        method: "GET",
        path: "/topics/{id}/schedule",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "get_wiki_pin",
        method: "GET",
        path: "/topics/{id}/wiki-pin",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "join_critique",
        method: "POST",
        path: "/topics/{id}/critique/join",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "thread_modes.rs",
        handler: "get_critique_queue",
        method: "GET",
        path: "/topics/{id}/critique/queue",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "community.rs",
        handler: "forum_search",
        method: "GET",
        path: "/forum-search",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "community.rs",
        handler: "subscribe_topic",
        method: "POST",
        path: "/topics/{id}/subscribe",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "unsubscribe_topic",
        method: "POST",
        path: "/topics/{id}/unsubscribe",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "mark_topic_read",
        method: "POST",
        path: "/topics/{id}/mark-read",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "community.rs",
        handler: "get_unread_count",
        method: "GET",
        path: "/topics/{id}/unread",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "list_lists",
        method: "GET",
        path: "/directory/lists",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "get_list",
        method: "GET",
        path: "/directory/lists/{slug}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "list_entries",
        method: "GET",
        path: "/directory/entries",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "get_entry",
        method: "GET",
        path: "/directory/entries/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "approve_entry",
        method: "POST",
        path: "/directory/entries/{id}/approve",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "vote",
        method: "POST",
        path: "/directory/entries/{id}/vote",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "categories",
        method: "GET",
        path: "/directory/categories",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "moderation_queue",
        method: "GET",
        path: "/directory/moderation",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "governance_state",
        method: "GET",
        path: "/directory/categories/governance",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "create_proposal",
        method: "POST",
        path: "/directory/categories/governance/proposals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "get_proposal",
        method: "GET",
        path: "/directory/categories/governance/proposals/{id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "vote_proposal",
        method: "POST",
        path: "/directory/categories/governance/proposals/{id}/vote",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "veto_proposal",
        method: "POST",
        path: "/directory/categories/governance/proposals/{id}/veto",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "changelog",
        method: "GET",
        path: "/directory/categories/governance/changelog/{slug}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "toggle_freeze",
        method: "POST",
        path: "/directory/categories/governance/freeze",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "set_max_categories",
        method: "POST",
        path: "/directory/categories/governance/max",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "propose_entry_mod",
        method: "POST",
        path: "/directory/entries/{id}/moderation",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "directory.rs",
        handler: "vote_entry_mod",
        method: "POST",
        path: "/directory/entries/{id}/moderation/vote",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "cta.rs",
        handler: "mark",
        method: "POST",
        path: "/works/{id}/cta_marks",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "arena.rs",
        handler: "get_arena_next",
        method: "GET",
        path: "/arena/next",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "arena.rs",
        handler: "post_arena_vote",
        method: "POST",
        path: "/arena/vote",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "arena.rs",
        handler: "post_arena_dismiss",
        method: "POST",
        path: "/arena/dismiss",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "arena.rs",
        handler: "get_arena_weights",
        method: "GET",
        path: "/arena/weights",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "get_board",
        method: "GET",
        path: "/roadmap",
        audience: Audience::Public,
    },
    // Public, like the board it belongs to. §44.5 makes prioritization public
    // and a card's body is part of what is public about a card: the arena asks
    // the community to rank features, so the reasoning behind each feature has
    // to be readable by the same people the arena is for. Gating the body
    // behind a session would hide it from exactly the audience that needs it
    // most, the anonymous reader deciding whether this is a project they want
    // to be on.
    RouteEntry {
        file: "roadmap.rs",
        handler: "get_card",
        method: "GET",
        path: "/roadmap/cards/{id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "get_changelog",
        method: "GET",
        path: "/roadmap/changelog",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "get_arena",
        method: "GET",
        path: "/roadmap/arena",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "post_suggest",
        method: "POST",
        path: "/roadmap/suggest",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "roadmap.rs",
        handler: "post_move_card",
        method: "POST",
        path: "/admin/roadmap/move",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "deliver_export",
        method: "POST",
        path: "/exports/{id}/deliver",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "fork_work",
        method: "POST",
        path: "/works/{id}/fork",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "audience.rs",
        handler: "get_audience",
        method: "GET",
        path: "/me/audience",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "put_spoiler_scope",
        method: "PUT",
        path: "/topics/{id}/spoiler-scope",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "get_progress",
        method: "GET",
        path: "/works/{id}/progress",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "get_warnings",
        method: "GET",
        path: "/posts/{id}/warnings",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "get_draft",
        method: "GET",
        path: "/topics/{id}/draft",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "post_schedule",
        method: "POST",
        path: "/posts/{id}/schedule",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "get_due_scheduled",
        method: "GET",
        path: "/posts/scheduled/due",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "post_publish_scheduled",
        method: "POST",
        path: "/posts/scheduled/{id}/publish",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "spoilers.rs",
        handler: "get_warning_prefs",
        method: "GET",
        path: "/me/warning-prefs",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "imports.rs",
        handler: "start_import_batch",
        method: "POST",
        path: "/imports/batch",
        audience: Audience::Pseudonymous,
    },
    // ------------------------------------------------------------------
    // Vanguard — session-scoped admin + role-gated writes (spec §16.18)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "vanguard.rs",
        handler: "get_my_vanguard_status",
        method: "GET",
        path: "/vanguard/status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "get_pins_for_work",
        method: "GET",
        path: "/vanguard/pins/{work_id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "pin_work",
        method: "POST",
        path: "/vanguard/pins/{work_id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "unpin_work",
        method: "DELETE",
        path: "/vanguard/pins/{work_id}",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "list_vanguards",
        method: "GET",
        path: "/vanguards",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "grant_vanguard",
        method: "POST",
        path: "/vanguards",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "vanguard.rs",
        handler: "revoke_vanguard",
        method: "DELETE",
        path: "/vanguards/{account_id}",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Media resilience — public reads, session writes, curator mirrors
    // (spec §32.7)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "media_resilience.rs",
        handler: "get_media_reference",
        method: "GET",
        path: "/media/references/{reference_id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "get_work_media_references",
        method: "GET",
        path: "/works/{work_id}/media",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "add_media_reference",
        method: "POST",
        path: "/works/{work_id}/media",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "report_broken_link",
        method: "POST",
        path: "/media/references/{reference_id}/report-broken",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "add_mirror_link",
        method: "POST",
        path: "/media/references/{reference_id}/mirrors",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "list_match_proposals",
        method: "GET",
        path: "/media/match-proposals",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "media_resilience.rs",
        handler: "resolve_match_proposal",
        method: "POST",
        path: "/media/match-proposals/{proposal_id}",
        audience: Audience::Operator,
    },
    // ------------------------------------------------------------------
    // Curator role (spec §32.7.5)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "curator.rs",
        handler: "opt_in_curator",
        method: "POST",
        path: "/curators/opt-in",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "opt_out_curator",
        method: "POST",
        path: "/curators/opt-out",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "get_my_curator_status",
        method: "GET",
        path: "/curators/status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "list_curators",
        method: "GET",
        path: "/curators",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "verify_link",
        method: "POST",
        path: "/media/references/{reference_id}/verify",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "get_quorum_status",
        method: "GET",
        path: "/media/references/{reference_id}/quorum",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "curator.rs",
        handler: "get_matching_bounties",
        method: "GET",
        path: "/curator/bounties",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Author media tools (spec §32.7.8)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "author_media.rs",
        handler: "get_preferences",
        method: "GET",
        path: "/author/media-preferences",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "author_media.rs",
        handler: "update_preferences",
        method: "PUT",
        path: "/author/media-preferences",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "author_media.rs",
        handler: "get_author_media_health",
        method: "GET",
        path: "/author/media-health",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "author_media.rs",
        handler: "post_targeted_bounty",
        method: "POST",
        path: "/author/targeted-bounties",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "author_media.rs",
        handler: "claim_targeted_bounty",
        method: "POST",
        path: "/author/targeted-bounties/claim",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "author_media.rs",
        handler: "list_targeted_bounties",
        method: "GET",
        path: "/works/{work_id}/targeted-bounties",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Mirror admin & IPFS (spec §32.7.6)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "add_local_mirror",
        method: "POST",
        path: "/admin/local-mirrors",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "deactivate_local_mirror",
        method: "DELETE",
        path: "/admin/local-mirrors/{mirror_id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "list_local_mirrors",
        method: "GET",
        path: "/media/references/{reference_id}/mirrors",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "file_dmca_takedown",
        method: "POST",
        path: "/admin/dmca-takedowns",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "list_dmca_takedowns",
        method: "GET",
        path: "/admin/dmca-takedowns",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "resolve_dmca_takedown",
        method: "PUT",
        path: "/admin/dmca-takedowns/{takedown_id}",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "add_ipfs_pin",
        method: "POST",
        path: "/media/references/{reference_id}/ipfs-pins",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "mirror_admin.rs",
        handler: "list_ipfs_pins",
        method: "GET",
        path: "/media/references/{reference_id}/ipfs-pins",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Media health dashboard (spec §32.7.11)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "media_health.rs",
        handler: "media_health_overview",
        method: "GET",
        path: "/admin/media-health/overview",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_health.rs",
        handler: "link_rot_report",
        method: "GET",
        path: "/admin/media-health/link-rot",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_health.rs",
        handler: "curator_leaderboard",
        method: "GET",
        path: "/admin/media-health/curator-leaderboard",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_health.rs",
        handler: "bounty_status",
        method: "GET",
        path: "/admin/media-health/bounty-status",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_health.rs",
        handler: "storage_status",
        method: "GET",
        path: "/admin/media-health/storage",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "media_health.rs",
        handler: "provider_reliability",
        method: "GET",
        path: "/admin/media-health/providers",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Discovery: reverse search & curator bounty queue (spec §32.7.3, §32.7.5)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "discovery.rs",
        handler: "reverse_search",
        method: "POST",
        path: "/media/reverse-search",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "curator_bounty_queue",
        method: "GET",
        path: "/curator/bounty-queue",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Discovery — additional routes not in main table (spec §16)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "discovery.rs",
        handler: "get_my_taste_vector",
        method: "GET",
        path: "/discovery/taste-profile/me",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_admin_taste_profile",
        method: "GET",
        path: "/operator/taste-profile",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "recompute_all_taste_profiles",
        method: "POST",
        path: "/operator/taste-profile/recompute-all",
        audience: Audience::Authenticated,
    },
    // M29 recommendation transparency. The explanations and the attention report
    // are about the reader, so a session is required and the reader's own pseud
    // scopes them -- there is no "someone else's explanation" to ask for.
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "set_attention_report",
        method: "PUT",
        path: "/me/attention-report",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "propose_wrangling",
        method: "POST",
        path: "/admin/tag-wrangling/proposals",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "list_wrangling_proposals",
        method: "GET",
        path: "/admin/tag-wrangling/proposals",
        audience: Audience::Operator,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "approve_wrangling",
        method: "POST",
        path: "/admin/tag-wrangling/proposals/{id}/approve",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "reject_wrangling",
        method: "POST",
        path: "/admin/tag-wrangling/proposals/{id}/reject",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "revert_wrangling",
        method: "POST",
        path: "/admin/tag-wrangling/proposals/{id}/revert",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "public_wrangling_log",
        method: "GET",
        path: "/tag-wrangling/log",
        // No session extractor on the handler at all, which is what a public
        // door looks like in this codebase. The inventory infers `MaybeSession`
        // from the absence, so that is what the table says.
        audience: Audience::Public,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "update_admin_taste_profile",
        method: "PUT",
        path: "/operator/taste-profile",
        audience: Audience::Authenticated,
    },
    // F1/F2: the taste knobs are a versioned object, so the history and the
    // rollback are operator doors like the settings they change.
    RouteEntry {
        file: "discovery.rs",
        handler: "get_admin_taste_profile_history",
        method: "GET",
        path: "/operator/taste-profile/history",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "rollback_admin_taste_profile",
        method: "POST",
        path: "/operator/taste-profile/history/{history_id}/rollback",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "discovery.rs",
        handler: "get_my_streak",
        method: "GET",
        path: "/me/streak",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // Reader surface — items 14, 27, 33 of the 100-idea audit.
    //
    // The three audiences differ, and the difference is the design:
    // `new_in_your_fandoms` answers "YOURS", so it needs a session; the other two
    // read only public rows and stay public, because a login wall in front of
    // published data teaches readers the data is not published — and it would
    // hide the store's is_public privacy rule from exactly the reader who could
    // check it.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "reader_surface.rs",
        handler: "new_in_your_fandoms",
        method: "GET",
        path: "/discovery/new-in-your-fandoms",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "reader_surface.rs",
        handler: "most_bookmarked",
        method: "GET",
        path: "/discovery/most-bookmarked",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_surface.rs",
        handler: "similar",
        method: "GET",
        path: "/works/{work_id}/similar",
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // The work's declared warnings (spec §10.2).
    //
    // A **contributor** door: a warning is a statement about the work's
    // content, and only the author can make that statement. A reader cannot
    // declare a warning on somebody else's work.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "works.rs",
        handler: "declare_warning",
        method: "POST",
        path: "/works/{id}/warnings",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // A unit's spoiler gate (spec §10.1).
    // ------------------------------------------------------------------
    RouteEntry {
        file: "works.rs",
        handler: "set_unit_gate",
        method: "POST",
        path: "/works/{id}/units/{unit}/gate",
        audience: Audience::Authenticated,
    },
    // ------------------------------------------------------------------
    // The reading surface — server-rendered HTML (spec §8.5, §10.1).
    //
    // These are PAGES and not API responses, so they are merged at the root
    // rather than nested under `/api/v1`. The audience is **Public**: a
    // signed-out visitor may read a published work, and a reader page that
    // asked for a session would be a login wall in front of the archive's own
    // content.
    // ------------------------------------------------------------------
    RouteEntry {
        file: "reader_page.rs",
        handler: "read_unit_page",
        method: "GET",
        path: "/works/{work_id}/units/{unit_id}",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_page.rs",
        handler: "save_typography",
        method: "POST",
        path: "/works/{work_id}/units/{unit_id}/typography",
        // **Public, and deliberately so** -- see `save_position` below.
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_page.rs",
        handler: "reveal_section",
        method: "POST",
        path: "/works/{work_id}/units/{unit_id}/reveal",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_page.rs",
        handler: "save_note",
        method: "POST",
        path: "/works/{work_id}/units/{unit_id}/notes",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_page.rs",
        handler: "delete_note",
        method: "POST",
        path: "/works/{work_id}/units/{unit_id}/notes/{note_id}/delete",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "reader_page.rs",
        handler: "save_position",
        method: "POST",
        path: "/works/{work_id}/units/{unit_id}/position",
        // **Public, and deliberately so.** The door is reachable by anyone:
        // a signed-out reader's post is redirected back to the page they were
        // on, because their place lives in their browser and not on an
        // account (spec §10.3), and a 401 would be a failure where the
        // specification asks for a degradation. "The position save degrades to
        // a form post, not a failure" — T-M5-01.
        audience: Audience::Public,
    },
    // ------------------------------------------------------------------
    // Monetization — additional routes not in main table (spec §20)
    // ------------------------------------------------------------------
    RouteEntry {
        file: "monetization.rs",
        handler: "set_work_ai_declaration",
        method: "POST",
        path: "/works/{work_id}/ai-declaration",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "get_transparency_dashboard",
        method: "GET",
        path: "/transparency/monetization",
        audience: Audience::Public,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "record_reading_session",
        method: "POST",
        path: "/works/{work_id}/reading-session",
        audience: Audience::Pseudonymous,
    },
    RouteEntry {
        file: "monetization.rs",
        handler: "settle_period",
        method: "POST",
        path: "/admin/monetization/settle",
        audience: Audience::Authenticated,
    },
];
