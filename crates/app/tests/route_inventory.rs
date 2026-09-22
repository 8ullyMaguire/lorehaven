//! Route-inventory audit — spec §2.3.
//!
//! Every route handler in the application must declare an audience
//! extractor (`MaybeSession` for public doors, `RequireSession` for
//! authenticated doors).  Handlers without either extractor are silent
//! defaults to anonymous-only and leak data to unauthenticated callers.
//!
// Phase 1c (§2.3c): extends the presence check to an explicit
//! `(method, path) → audience` table.  The test now:
//!   1. Scans every route source file for `async fn` with
//!      `State<AppState>` and records its declared extractor
//!      (`MaybeSession`, `RequireSession`, `RequirePseud`, or none).
//!   2. Compares each found handler against the table below; the
//!      table entry names the file, the handler, and the *correct*
//!      audience.  A mismatch (wrong extractor type, or missing
//!      extractor entirely) is a build failure.
//!   3. Asserts every registered route in `server.rs` has a
//!      corresponding table entry — no unregistered route may exist.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The audience a handler must declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Audience {
    /// Public door — `MaybeSession` (anonymous may reach it).
    Public,
    /// Authenticated door — `RequireSession`.
    Authenticated,
    /// Pseudonymous door — `RequirePseud`.
    Pseudonymous,
    /// Operator door — `RequireSession` + `require_operator`.
    Operator,
}

impl Audience {
    fn as_str(&self) -> &'static str {
        match self {
            Audience::Public => "MaybeSession",
            Audience::Authenticated => "RequireSession",
            Audience::Pseudonymous => "RequirePseud",
            Audience::Operator => "RequireSession",
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
struct RouteEntry {
    file: &'static str,
    handler: &'static str,
    method: &'static str,
    path: &'static str,
    audience: Audience,
}

/// The complete declared set.  Every route the server registers must
/// appear here with its correct audience.  This table is the spec
/// answer to N2: the harness now measures the *right* audience, not
/// just any audience.
///
/// Populated from `server.rs` route registrations and verified
/// against each module's handler signatures.
const ROUTE_TABLE: &[RouteEntry] = &[
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
        handler: "add_chapter",
        method: "POST",
        path: "/works/{id}/chapters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "reorder_chapters",
        method: "POST",
        path: "/works/{id}/reorder-chapters",
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "works.rs",
        handler: "update_chapter",
        method: "PATCH",
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
        handler: "read_chapter",
        method: "GET",
        path: "/works/{id}/chapters/{chapter}",
        audience: Audience::Public,
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
        audience: Audience::Public,
    },
    RouteEntry {
        file: "recommendation_transparency.rs",
        handler: "get_attention_report",
        method: "GET",
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
        audience: Audience::Authenticated,
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
        audience: Audience::Authenticated,
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
        audience: Audience::Pseudonymous,
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
        audience: Audience::Authenticated,
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
        handler: "retry_failed_chapters",
        method: "POST",
        path: "/imports/{id}/retry-failed-chapters",
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
        audience: Audience::Authenticated,
    },
    RouteEntry {
        file: "exports.rs",
        handler: "get_export",
        method: "GET",
        path: "/exports/{id}",
        audience: Audience::Authenticated,
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
        audience: Audience::Authenticated,
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
        handler: "set_operator_affinity",
        method: "POST",
        path: "/operator/affinities",
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
        method: "PATCH",
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
        file: "cta.rs",
        handler: "retract",
        method: "GET",
        path: "/works/{id}/cta_marks/me",
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
    RouteEntry {
        file: "discovery.rs",
        handler: "get_my_streak",
        method: "GET",
        path: "/me/streak",
        audience: Audience::Authenticated,
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

/// Check whether a function signature contains an audience extractor
/// (`MaybeSession`, `RequireSession`, or `RequirePseud`), and return
/// which one it found (or `None`).
fn find_audience_extractor(sig: &str) -> Option<&'static str> {
    let args_start = sig.find('(')?;
    let mut depth = 0u32;
    let mut args_end = args_start;
    for (i, ch) in sig[args_start..].chars().enumerate() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    args_end = args_start + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let args = &sig[args_start..args_end];
    if args.contains("RequirePseud") {
        Some("RequirePseud")
    } else if args.contains("RequireSession") {
        Some("RequireSession")
    } else if args.contains("MaybeSession") {
        Some("MaybeSession")
    } else {
        None
    }
}

/// Collect a multi-line function signature starting at `start_line`.
/// Stops when the parenthesis depth returns to zero after the
/// opening `(`.
fn collect_signature(src: &str, start_line: usize) -> String {
    let mut sig = String::new();
    let mut depth = 0u32;
    let mut found_open_paren = false;
    for line in src.lines().skip(start_line) {
        for ch in line.chars() {
            match ch {
                '(' | '[' | '{' => {
                    depth += 1;
                    found_open_paren = true;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                }
                _ => {}
            }
            sig.push(ch);
            if found_open_paren && depth == 0 {
                return sig;
            }
        }
        if found_open_paren && depth == 0 {
            return sig;
        }
    }
    sig
}

/// Verify every handler declared in `ROUTE_TABLE` actually exists in
/// the source file with the correct audience extractor.
#[test]
fn every_route_has_correct_audience() {
    // Build a lookup from (file, handler) to the expected audience.
    let mut expected: BTreeMap<(&str, &str), (&Audience, &str, &str)> = BTreeMap::new();
    for entry in ROUTE_TABLE {
        expected.insert(
            (entry.file, entry.handler),
            (&entry.audience, entry.method, entry.path),
        );
    }

    let routes_dir = Path::new("src/routes");
    let mut failures: Vec<String> = Vec::new();

    for entry in ROUTE_TABLE {
        let src_path = routes_dir.join(entry.file);
        let src = fs::read_to_string(&src_path)
            .unwrap_or_else(|_| panic!("cannot read {}: {}", entry.file, src_path.display()));

        // Find the handler function and check its extractor.
        let mut found = false;
        for (line_no, line) in src.lines().enumerate() {
            let trimmed = line.trim();
            if !trimmed.starts_with("async fn") && !trimmed.starts_with("pub async fn") {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            let fn_name = parts
                .iter()
                .rposition(|p| *p == "fn")
                .and_then(|i| parts.get(i + 1))
                .map(|p| {
                    // Extract the function name: take up to the first '(' if present
                    p.split('(').next().unwrap_or(p).trim()
                })
                .unwrap_or("");
            if fn_name != entry.handler {
                continue;
            }
            let sig = collect_signature(&src, line_no);
            if !sig.contains("State<AppState>") {
                continue;
            }
            found = true;
            let actual = find_audience_extractor(&sig);
            let expected_str = entry.audience.as_str();
            match actual {
                Some(actual_str) if actual_str == expected_str => {}
                Some(actual_str) => {
                    failures.push(format!(
                        "{}:{} — {} expected {} but found {}",
                        entry.file,
                        line_no + 1,
                        entry.handler,
                        expected_str,
                        actual_str
                    ));
                }
                None => {
                    failures.push(format!(
                        "{}:{} — {} has no audience extractor (expected {})",
                        entry.file,
                        line_no + 1,
                        entry.handler,
                        expected_str
                    ));
                }
            }
            break;
        }
        if !found {
            failures.push(format!(
                "{}:{} — handler not found with State<AppState>",
                entry.file, entry.handler
            ));
        }
    }

    if !failures.is_empty() {
        panic!("Audience mismatches:\n{}", failures.join("\n"));
    }
}

/// Verify the table covers every registered route — no unregistered
/// route may exist in `server.rs`.
///
/// We check each route module's `router()` function for the path
/// The name of the nearest `fn` declared at or above `index` — the function a
/// `.route(...)` call on that line belongs to.
fn enclosing_fn(lines: &[&str], index: usize) -> Option<(String, bool)> {
    for line in lines[..=index].iter().rev() {
        let t = line.trim_start();
        if let Some(i) = t.find("fn ") {
            let name: String = t[i + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                // A router builder is any function returning `Router<…>`:
                // `router()`, `routes()`, `write_router()`, `*_routes()`.  The
                // name alone is not enough — `governance.rs` declares
                // `fn router() { routes() }` and registers its doors in
                // `fn routes()`, which a name list of "router"/"*_routes"
                // silently skipped.
                return Some((name, line.contains("Router<")));
            }
        }
    }
    None
}

/// Walk a module's `router()` and `*_routes()` functions, resolve `.nest()`
/// prefixes, and collect every `(full_path, method, handler)` triple that the
/// module registers.
///
/// This is the *direction* test: it walks the code, not the table.  Any
/// handler registered in a module but missing from `ROUTE_TABLE` is caught
/// here, including routes inside nested sub-routers.
fn collect_registered(module: &str) -> Vec<(String, String, String)> {
    let src_path = Path::new("src/routes").join(module);
    let src = fs::read_to_string(&src_path)
        .unwrap_or_else(|_| panic!("cannot read {}: {}", module, src_path.display()));
    let mut routes = Vec::new();

    // Associate every `.route(...)` with the nearest `fn` declared above it, and
    // keep the ones registered in a `router()` / `*_routes()` function.
    //
    // This replaced a brace-counting scanner that walked function bodies. That
    // scanner could not be trusted: braces inside strings and comments moved the
    // depth, so a body could end early or swallow the rest of the file, and the
    // failure mode was silence — it collected 0 routes from all 33 modules and
    // this test still passed. Scanning backwards for the enclosing declaration
    // has no depth to get wrong.
    let lines: Vec<&str> = src.lines().collect();
    for (idx, line) in lines.iter().enumerate() {
        if !line.contains(".route(") {
            continue;
        }
        let Some((func, builds_router)) = enclosing_fn(&lines, idx) else {
            continue;
        };
        if !builds_router {
            continue;
        }
        let prefix = find_nest_prefix(&src, module, &func);
        let t = line.trim();

        // .route("path", handler) or .route("path", get(handler))
        if let Some(route_start) = t.find(".route(") {
            let after_route = &t[route_start + 7..];
            if let Some(path_end) = after_route.find('"') {
                let rest = &after_route[path_end + 1..];
                if let Some(path_close) = rest.find('"') {
                    let path = &rest[..path_close];
                    let after_path = &rest[path_close + 1..];

                    // Find handler in the rest: get(handler), post(handler), etc.
                    if let Some(handler) = extract_handler(after_path) {
                        let full_path = if prefix.is_empty() {
                            path.to_string()
                        } else if path == "/" {
                            // A nested router's own root is spelled with the
                            // trailing slash the table uses (`/recipes/`).
                            format!("{prefix}/")
                        } else {
                            format!("{}{}", prefix, path)
                        };
                        routes.push((full_path, handler, func.clone()));
                    }
                }
            }
        }

        // .nest("prefix", func()) — handled by `find_nest_prefix`.
    }

    routes
}

/// Find the `.nest("prefix", func_name())` call that nests this module's
/// sub-router under a prefix.  Called from the parent router.
fn find_nest_prefix(src: &str, _module: &str, func_name: &str) -> String {
    for line in src.lines() {
        let t = line.trim();
        if t.contains(".nest(") && t.contains(func_name) {
            if let Some(nest_start) = t.find(".nest(") {
                let after = &t[nest_start + 6..];
                if let Some(p_start) = after.find('"') {
                    let rest = &after[p_start + 1..];
                    if let Some(p_end) = rest.find('"') {
                        return rest[..p_end].to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// Extract the handler function name from the rest of a .route() call
/// after the path, e.g. `, get(list_media))` → `list_media`
fn extract_handler(s: &str) -> Option<String> {
    // What follows the path is `, get(handler))` — with a leading comma, which
    // the previous implementation treated as end-of-route and gave up on, so
    // every route in every module was skipped and this test passed while
    // collecting nothing.
    let s = s.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    let open = s.find('(')?;
    let inner = &s[open + 1..];
    let close = inner.find([')', ','])?;
    let name = inner[..close].trim();
    // A method chain (`get(a).post(b)`) contributes its first handler only; the
    // rest are separate registrations and are not collected today.
    (!name.is_empty() && !name.contains('(')).then(|| name.to_string())
}

/// The direction test: walk every module's router() and *_routes() to find
/// all registered paths, then verify each has a row in ROUTE_TABLE.
#[test]
fn registered_routes_are_tabled() {
    let routes_dir = Path::new("src/routes");
    let mut failures: Vec<String> = Vec::new();

    // Build a set of all registered (file, path, method, handler) triples from the table
    let mut table_entries: Vec<(String, String, String, String)> = Vec::new();
    for entry in ROUTE_TABLE {
        table_entries.push((
            entry.file.to_string(),
            entry.path.to_string(),
            entry.method.to_string(),
            entry.handler.to_string(),
        ));
    }

    // Walk every module file
    let entries = fs::read_dir(routes_dir).expect("cannot read routes directory");
    for entry in entries {
        let entry = entry.expect("read_dir entry");
        let path = entry.path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == "rs") {
            continue;
        }
        let module = path.file_stem().unwrap().to_string_lossy().to_string() + ".rs";
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();

        // Skip lib.rs and non-router files
        if file_name == "lib.rs" {
            continue;
        }

        let registered = collect_registered(&module);
        // A walk that collects nothing compares nothing and passes. Every module
        // here declares a router function, so an empty result means the walk
        // broke, not that the module registers no routes — which is exactly how
        // this test passed while collecting 0 routes from all 33 modules.
        let src = fs::read_to_string(&path).expect("read module source");
        if registered.is_empty() && src.contains("Router<") {
            failures.push(format!(
                "{} — declares a router function but the walk collected no routes \
                 (the walk is broken, not the module)",
                file_name
            ));
        }
        for (full_path, handler, _func) in registered {
            // Find matching table entry: same file, path, method, handler
            let found = table_entries
                .iter()
                .any(|(f, p, _m, h)| f == &file_name && p == &full_path && h == &handler);
            if !found {
                failures.push(format!(
                    "{}:{} — handler '{}' registered but not in ROUTE_TABLE (path {})",
                    file_name, full_path, handler, full_path
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!("Unregistered routes:\n{}", failures.join("\n"));
    }
}
