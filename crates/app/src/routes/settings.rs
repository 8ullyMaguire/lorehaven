//! Settings endpoints (spec §7).
//!
//! ```text
//! GET   /settings/privacy    PATCH /settings/privacy
//! GET   /settings/content    PATCH /settings/content
//! ```
//!
//! Privacy and content are separate endpoints because they answer different
//! questions — "who may see what of mine" versus "what do I want to be shown" —
//! and because they have different ceilings. A privacy change can narrow
//! exposure on its own; a content change is always intersected with what the
//! instance's policy allows, so setting `max_rating: explicit` as a restricted
//! account widens nothing.

use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db::identity::{self, PrivacyScope};
use lorehaven_db::sessions;
use lorehaven_db::settings as db_settings;
use lorehaven_domain::policy::ContentRating;
use lorehaven_domain::settings::{
    self as domain_settings, resolve_setting, ResolvedSetting, SettingsExport,
    SETTINGS_EXPORT_VERSION,
};
use lorehaven_domain::{AppError, PseudId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::RequireSession;
use crate::http::{ApiError, ApiResult};
use crate::privacy;
use crate::state::AppState;

/// Settings routes.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/settings/privacy", get(get_privacy).patch(patch_privacy))
        .route("/settings/content", get(get_content).patch(patch_content))
        // M47: per-namespace settings endpoints (spec §46.3).
        .route(
            "/settings/search",
            get(get_search_settings).patch(patch_search_settings),
        )
        .route(
            "/settings/search/{key}",
            axum::routing::delete(delete_search_setting),
        )
        .route(
            "/settings/content-filters",
            get(list_content_filters).post(post_content_filter),
        )
        .route(
            "/settings/content-filters/{filter_type}/{value}",
            axum::routing::delete(delete_content_filter),
        )
        .route(
            "/settings/notifications",
            get(get_notification_routes).patch(patch_notification_route),
        )
        .route(
            "/settings/notifications/{event_type}",
            axum::routing::delete(delete_notification_route),
        )
        .route("/settings/export", get(export_settings))
        .route("/settings/import", post(import_settings))
}

// ---------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PrivacyView {
    /// Account-level settings.
    account: BTreeMap<String, String>,
    /// Pseud-level settings, keyed by pseud identifier.
    pseuds: BTreeMap<String, BTreeMap<String, String>>,
    /// Every recognised key, its permitted values and its description, so the
    /// interface renders what the server accepts rather than maintaining its
    /// own copy that can drift.
    schema: Vec<KeyDescription>,
}

#[derive(Debug, Serialize)]
struct KeyDescription {
    key: &'static str,
    summary: &'static str,
    values: &'static [&'static str],
}

#[derive(Debug, Deserialize)]
struct PatchPrivacy {
    /// Pseud-level setting to change. Absent means account-level.
    #[serde(default)]
    pseud_id: Option<PseudId>,
    /// The settings to change.
    changes: BTreeMap<String, String>,
}

async fn get_privacy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<PrivacyView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;

    let mut account_settings = BTreeMap::new();
    for key in privacy::keys_in(privacy::Scope::Account) {
        // A stored value is used; otherwise the default for this account's age
        // state is reported. The two are distinguished nowhere in the response
        // on purpose: a client should not be able to tell the difference
        // between "chose the default" and "never chose", because that
        // difference is not actionable.
        let stored = identity::privacy_value(
            state.db(),
            PrivacyScope::Account(&user.account_id),
            key.name,
        )
        .await?;
        account_settings.insert(
            key.name.to_owned(),
            stored.unwrap_or_else(|| privacy::default_for(key.name, account.age_state).to_owned()),
        );
    }

    let mut pseud_settings = BTreeMap::new();
    for pseud in identity::pseuds_for_account(state.db(), user.account_id).await? {
        let mut values = BTreeMap::new();
        for key in privacy::keys_in(privacy::Scope::Pseud) {
            let stored =
                identity::privacy_value(state.db(), PrivacyScope::Pseud(&pseud.id), key.name)
                    .await?;
            values.insert(
                key.name.to_owned(),
                stored.unwrap_or_else(|| {
                    privacy::default_for(key.name, account.age_state).to_owned()
                }),
            );
        }
        pseud_settings.insert(pseud.id.to_string(), values);
    }

    Ok(Json(PrivacyView {
        account: account_settings,
        pseuds: pseud_settings,
        schema: privacy::ALL
            .iter()
            .map(|key| KeyDescription {
                key: key.name,
                summary: key.summary,
                values: key.values,
            })
            .collect(),
    }))
}

async fn patch_privacy(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchPrivacy>,
) -> ApiResult<Json<PrivacyView>> {
    if request.changes.is_empty() {
        return Err(ApiError(AppError::field(
            "changes",
            "Supply at least one setting to change.",
        )));
    }

    // Validate everything before writing anything: a request that names one
    // good key and one bad one must change nothing.
    for (key, value) in &request.changes {
        let Some(description) = privacy::find(key) else {
            return Err(ApiError(AppError::field(
                key,
                "That is not a recognised privacy setting.",
            )));
        };
        if !privacy::is_valid_value(description, value) {
            return Err(ApiError(AppError::field(
                key,
                format!("{} must be one of: {}.", key, description.values.join(", ")),
            )));
        }
    }

    // Each key belongs to exactly one scope, declared once in `privacy`. A
    // request that names a key at the wrong scope is refused rather than
    // reinterpreted, because guessing would let the same key mean two things.
    let target_scope = if request.pseud_id.is_some() {
        privacy::Scope::Pseud
    } else {
        privacy::Scope::Account
    };

    for key in request.changes.keys() {
        let declared = privacy::find(key).expect("validated above");
        if declared.scope != target_scope {
            let hint = match declared.scope {
                privacy::Scope::Pseud => "This setting applies to one pseud; supply pseud_id.",
                privacy::Scope::Account => "This setting applies to the account, not to one pseud.",
            };
            return Err(ApiError(AppError::field(key, hint)));
        }
    }

    match request.pseud_id {
        Some(pseud_id) => {
            // Ownership check: the pseud must belong to the caller.
            let owned = identity::find_pseud(state.db(), pseud_id)
                .await?
                .is_some_and(|pseud| pseud.account_id == user.account_id);
            if !owned {
                return Err(ApiError(AppError::NotFound { resource: "pseud" }));
            }

            for (key, value) in &request.changes {
                identity::set_privacy(state.db(), PrivacyScope::Pseud(&pseud_id), key, value)
                    .await?;
            }
        }
        None => {
            for (key, value) in &request.changes {
                identity::set_privacy(
                    state.db(),
                    PrivacyScope::Account(&user.account_id),
                    key,
                    value,
                )
                .await?;
            }
        }
    }

    // Return the resulting state, so the client does not have to guess what a
    // partial update produced.
    get_privacy(State(state), RequireSession(user)).await
}

// ---------------------------------------------------------------------------
// Content
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ContentView {
    /// The rating the reader has chosen.
    max_rating: &'static str,
    /// Warnings the reader never wants to see.
    excluded_warnings: Vec<String>,
    /// The highest rating the instance's policy will show this account,
    /// whatever they choose. Reported so the interface can explain why a
    /// higher preference had no effect.
    policy_ceiling: &'static str,
    /// The rating actually in force: the lower of the two.
    effective_max_rating: &'static str,
    /// Optimistic-concurrency version, required by the next `PATCH`.
    version: i64,
}

#[derive(Debug, Deserialize)]
struct PatchContent {
    /// The version the client believes it is editing (spec §3.4).
    expected_version: i64,
    #[serde(default)]
    max_rating: Option<String>,
    #[serde(default)]
    excluded_warnings: Option<Vec<String>>,
}

async fn get_content(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ContentView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let settings = sessions::content_settings(state.db(), user.account_id).await?;

    Ok(Json(content_view(account.age_state, &settings)))
}

fn content_view(
    age_state: lorehaven_domain::policy::AgeState,
    settings: &sessions::ContentSettings,
) -> ContentView {
    let policy = lorehaven_domain::policy::AccessPolicy::default();
    let ceiling = policy_ceiling(age_state, &policy);

    ContentView {
        max_rating: sessions::rating_name(settings.max_rating),
        excluded_warnings: settings.excluded_warnings.clone(),
        policy_ceiling: sessions::rating_name(ceiling),
        effective_max_rating: sessions::rating_name(ceiling.min(settings.max_rating)),
        version: settings.version,
    }
}

fn policy_ceiling(
    age_state: lorehaven_domain::policy::AgeState,
    policy: &lorehaven_domain::policy::AccessPolicy,
) -> ContentRating {
    match age_state {
        lorehaven_domain::policy::AgeState::DeclaredAdult => policy.adult_max_rating,
        _ => policy.minor_max_rating,
    }
}

async fn patch_content(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchContent>,
) -> ApiResult<Json<ContentView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let mut settings = sessions::content_settings(state.db(), user.account_id).await?;

    if settings.version != request.expected_version {
        return Err(ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: settings.version,
        }));
    }

    if let Some(rating) = &request.max_rating {
        if !matches!(rating.as_str(), "general" | "teen" | "mature" | "explicit") {
            return Err(ApiError(AppError::field(
                "max_rating",
                "Choose one of: general, teen, mature, explicit.",
            )));
        }
        // Stored as chosen, not clamped: the ceiling is applied on read, so a
        // reader who later becomes eligible for more does not have to come back
        // and change this again.
        settings.max_rating = sessions::parse_rating(rating);
    }

    if let Some(warnings) = request.excluded_warnings {
        if warnings.len() > 200 {
            return Err(ApiError(AppError::field(
                "excluded_warnings",
                "At most 200 warnings may be excluded.",
            )));
        }
        let mut cleaned = Vec::with_capacity(warnings.len());
        for warning in warnings {
            let trimmed = warning.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.chars().count() > 80 {
                return Err(ApiError(AppError::field(
                    "excluded_warnings",
                    "Each warning may be at most 80 characters.",
                )));
            }
            if trimmed.chars().any(char::is_control) {
                return Err(ApiError(AppError::field(
                    "excluded_warnings",
                    "A warning cannot contain control characters.",
                )));
            }
            cleaned.push(trimmed.to_owned());
        }
        cleaned.sort_unstable();
        cleaned.dedup();
        settings.excluded_warnings = cleaned;
    }

    sessions::save_content_settings(
        state.db(),
        user.account_id,
        &settings,
        Some(request.expected_version),
    )
    .await
    .map_err(|error| {
        // The repository refuses when the version moved between our read and
        // our write; surface that as the documented conflict rather than a
        // generic fault.
        ApiError(AppError::RevisionConflict {
            expected: request.expected_version,
            actual: request.expected_version + 1,
        })
        .tap(error)
    })?;

    let updated = sessions::content_settings(state.db(), user.account_id).await?;
    Ok(Json(content_view(account.age_state, &updated)))
}

/// Small helper so a mapped error can still be logged with its cause.
trait TapError<T> {
    fn tap(self, cause: anyhow::Error) -> T;
}

impl TapError<ApiError> for ApiError {
    fn tap(self, cause: anyhow::Error) -> ApiError {
        tracing::warn!(%cause, "content settings write refused");
        self
    }
}

// ---------------------------------------------------------------------------
// M47: User Configuration API family (spec §46)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ResolvedSettingView {
    key: String,
    value: serde_json::Value,
    source: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct SettingWriteItem {
    key: String,
    value: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct SearchSettingsView {
    pseud_id: String,
    settings: Vec<ResolvedSettingView>,
    schema: Vec<KeyDescription>,
}

#[derive(Debug, Deserialize)]
struct PatchSearchChanges {
    changes: Vec<SettingWriteItem>,
}

fn now_string() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn search_setting_schema() -> Vec<KeyDescription> {
    domain_settings::SETTING_KEYS
        .iter()
        .filter(|def| matches!(def.namespace, domain_settings::SettingNamespace::Search))
        .map(|def| KeyDescription {
            key: def.key,
            summary: def.summary,
            values: &[],
        })
        .collect()
}

async fn get_search_settings(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<SearchSettingsView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let rows = db_settings::read_search_settings(state.db(), pseud_id).await?;
    let schema_keys: Vec<&str> = domain_settings::SETTING_KEYS
        .iter()
        .filter(|def| matches!(def.namespace, domain_settings::SettingNamespace::Search))
        .map(|def| def.key)
        .collect();
    let mut settings = Vec::new();
    for key in schema_keys {
        let pseud_value = rows.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let resolved = resolve_setting(key, None, pseud_value, None)?;
        settings.push(ResolvedSettingView {
            key: resolved.key,
            value: resolved.value,
            source: format!("{:?}", resolved.source).to_lowercase(),
            summary: resolved.summary,
        });
    }
    Ok(Json(SearchSettingsView {
        pseud_id: pseud_id.to_string(),
        settings,
        schema: search_setting_schema(),
    }))
}

async fn patch_search_settings(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchSearchChanges>,
) -> ApiResult<Json<SearchSettingsView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let now = now_string();
    for item in &request.changes {
        if domain_settings::key_def(&item.key).is_none() {
            return Err(ApiError(AppError::field(
                "key",
                format!("unknown setting key: {}", item.key),
            )));
        }
        db_settings::upsert_search_setting(state.db(), pseud_id, &item.key, &item.value, &now)
            .await?;
    }
    get_search_settings(State(state), RequireSession(user)).await
}

async fn delete_search_setting(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let removed = db_settings::delete_search_setting(state.db(), pseud_id, &key).await?;
    Ok(Json(serde_json::json!({ "removed": removed, "key": key })))
}

#[derive(Debug, Serialize)]
struct ContentFilterListView {
    pseud_id: String,
    filters: Vec<ContentFilterView>,
}

#[derive(Debug, Serialize)]
struct ContentFilterView {
    filter_type: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct CreateContentFilter {
    filter_type: String,
    value: String,
}

async fn list_content_filters(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<ContentFilterListView>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let rows = db_settings::list_content_filters(state.db(), pseud_id).await?;
    Ok(Json(ContentFilterListView {
        pseud_id: pseud_id.to_string(),
        filters: rows
            .into_iter()
            .map(|r| ContentFilterView {
                filter_type: r.filter_type,
                value: r.value,
            })
            .collect(),
    }))
}

async fn post_content_filter(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<CreateContentFilter>,
) -> ApiResult<Json<ContentFilterView>> {
    if domain_settings::ContentFilterType::from_str(&request.filter_type).is_err() {
        return Err(ApiError(AppError::field(
            "filter_type",
            "unknown content filter type",
        )));
    }
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let now = now_string();
    db_settings::add_content_filter(
        state.db(),
        pseud_id,
        &request.filter_type,
        &request.value,
        &now,
    )
    .await?;
    Ok(Json(ContentFilterView {
        filter_type: request.filter_type,
        value: request.value,
    }))
}

async fn delete_content_filter(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Path((filter_type, value)): axum::extract::Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = identity::find_account(state.db(), user.account_id)
        .await?
        .ok_or_else(|| ApiError(AppError::AuthRequired))?;
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| account.id.as_uuid());
    let removed =
        db_settings::remove_content_filter(state.db(), pseud_id, &filter_type, &value).await?;
    Ok(Json(
        serde_json::json!({ "removed": removed, "filter_type": filter_type, "value": value }),
    ))
}

#[derive(Debug, Serialize)]
struct NotificationRoutesView {
    account_id: String,
    routes: Vec<NotificationRouteView>,
}

#[derive(Debug, Serialize)]
struct NotificationRouteView {
    event_type: String,
    channel: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct PatchNotificationRoutes {
    changes: Vec<NotificationRouteWrite>,
}

#[derive(Debug, Deserialize)]
struct NotificationRouteWrite {
    event_type: String,
    channel: String,
    enabled: bool,
}

async fn get_notification_routes(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<NotificationRoutesView>> {
    let account_id = user.account_id.as_uuid();
    let rows = db_settings::read_notification_routes(state.db(), account_id).await?;
    Ok(Json(NotificationRoutesView {
        account_id: user.account_id.to_string(),
        routes: rows
            .into_iter()
            .map(|r| NotificationRouteView {
                event_type: r.event_type,
                channel: r.channel,
                enabled: r.enabled,
            })
            .collect(),
    }))
}

async fn patch_notification_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<PatchNotificationRoutes>,
) -> ApiResult<Json<NotificationRoutesView>> {
    let account_id = user.account_id.as_uuid();
    let now = now_string();
    for item in &request.changes {
        db_settings::upsert_notification_route(
            state.db(),
            account_id,
            &item.event_type,
            &item.channel,
            item.enabled,
            &now,
        )
        .await?;
    }
    get_notification_routes(State(state), RequireSession(user)).await
}

async fn delete_notification_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    axum::extract::Path(event_type): axum::extract::Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.as_uuid();
    let removed = db_settings::delete_notification_route(state.db(), account_id, &event_type).await?;
    Ok(Json(serde_json::json!({ "removed": removed, "event_type": event_type })))
}

async fn export_settings(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<SettingsExport>> {
    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| user.account_id.as_uuid());

    let search_settings = db_settings::read_search_settings(state.db(), pseud_id).await?;
    let content_filters = db_settings::list_content_filters(state.db(), pseud_id).await?;
    let notification_routes = db_settings::read_notification_routes(state.db(), user.account_id.as_uuid()).await?;

    let mut namespaces: HashMap<String, Vec<ResolvedSetting>> = HashMap::new();

    use domain_settings::SettingSource;

    namespaces.insert(
        "search".to_string(),
        search_settings
            .into_iter()
            .map(|(key, value)| ResolvedSetting {
                key,
                value,
                source: SettingSource::Pseud,
                summary: String::new(),
            })
            .collect(),
    );

    namespaces.insert(
        "content_filters".to_string(),
        content_filters
            .into_iter()
            .map(|r| ResolvedSetting {
                key: r.filter_type,
                value: serde_json::Value::String(r.value),
                source: SettingSource::Pseud,
                summary: String::new(),
            })
            .collect(),
    );

    namespaces.insert(
        "notifications".to_string(),
        notification_routes
            .into_iter()
            .map(|r| ResolvedSetting {
                key: format!("{}:{}", r.event_type, r.channel),
                value: serde_json::Value::Bool(r.enabled),
                source: SettingSource::Account,
                summary: String::new(),
            })
            .collect(),
    );

    Ok(Json(SettingsExport {
        version: SETTINGS_EXPORT_VERSION,
        exported_at: now_string(),
        namespaces,
    }))
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    data: SettingsExport,
}

#[derive(Debug, Serialize)]
struct ImportReport {
    accepted: Vec<String>,
    rejected: Vec<ImportRejection>,
}

#[derive(Debug, Serialize)]
struct ImportRejection {
    key: String,
    reason: String,
}

async fn import_settings(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(request): Json<ImportRequest>,
) -> ApiResult<Json<ImportReport>> {
    if request.data.version != SETTINGS_EXPORT_VERSION {
        return Err(ApiError(AppError::field(
            "version",
            format!("unsupported export version: {}", request.data.version),
        )));
    }

    let pseud_id = user
        .pseud_id
        .as_ref()
        .map(|p| p.as_uuid())
        .unwrap_or_else(|| user.account_id.as_uuid());
    let account_id = user.account_id.as_uuid();

    let mut accepted = Vec::new();
    let mut rejected = Vec::new();

    let now = chrono::Utc::now().to_rfc3339();

    for (namespace, settings) in &request.data.namespaces {
        for setting in settings {
            let result = match namespace.as_str() {
                "search" => {
                    db_settings::upsert_search_setting(
                        state.db(),
                        pseud_id,
                        &setting.key,
                        &setting.value,
                        &now,
                    )
                    .await
                    .map_err(|e| e.to_string())
                }
                "content_filters" => {
                    let value = setting
                        .value
                        .as_str()
                        .ok_or_else(|| "filter value must be a string".to_string())
                        .map_err(|e| ApiError(AppError::field("value", e)))?;
                    db_settings::add_content_filter(
                        state.db(),
                        pseud_id,
                        &setting.key,
                        value,
                        &now,
                    )
                    .await
                    .map_err(|e| e.to_string())
                }
                "notifications" => {
                    let parts: Vec<&str> = setting.key.splitn(2, ':').collect();
                    if parts.len() != 2 {
                        Err(format!("invalid notification key: {}", setting.key))
                    } else {
                        let enabled = setting
                            .value
                            .as_bool()
                            .ok_or_else(|| "notification value must be a boolean".to_string())
                            .map_err(|e| ApiError(AppError::field("value", e)))?;
                        db_settings::upsert_notification_route(
                            state.db(),
                            account_id,
                            parts[0],
                            parts[1],
                            enabled,
                            &now,
                        )
                        .await
                        .map_err(|e| e.to_string())
                    }
                }
                _ => Err(format!("unknown namespace: {}", namespace)),
            };

            match result {
                Ok(_) => accepted.push(format!("{}/{}", namespace, setting.key)),
                Err(reason) => rejected.push(ImportRejection {
                    key: format!("{}/{}", namespace, setting.key),
                    reason,
                }),
            }
        }
    }

    Ok(Json(ImportReport { accepted, rejected }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorehaven_domain::policy::AgeState;

    #[test]
    fn the_policy_ceiling_follows_the_age_state() {
        let policy = lorehaven_domain::policy::AccessPolicy::default();
        assert_eq!(
            policy_ceiling(AgeState::DeclaredAdult, &policy),
            ContentRating::Explicit
        );
        assert_eq!(
            policy_ceiling(AgeState::DeclaredMinor, &policy),
            ContentRating::General
        );
        assert_eq!(
            policy_ceiling(AgeState::Unknown, &policy),
            ContentRating::General
        );
    }

    #[test]
    fn a_preference_above_the_ceiling_has_no_effect() {
        // The reader asks for explicit; the policy says general. The effective
        // value must be the policy's, or the ceiling is decoration.
        let settings = sessions::ContentSettings {
            max_rating: ContentRating::Explicit,
            excluded_warnings: Vec::new(),
            version: 1,
        };
        let view = content_view(AgeState::DeclaredMinor, &settings);

        assert_eq!(view.max_rating, "explicit", "the stored choice is reported");
        assert_eq!(view.policy_ceiling, "general");
        assert_eq!(
            view.effective_max_rating, "general",
            "the effective value must not exceed the policy ceiling"
        );
    }

    #[test]
    fn an_adult_may_widen_up_to_the_policy() {
        let settings = sessions::ContentSettings {
            max_rating: ContentRating::Mature,
            excluded_warnings: vec!["character death".to_owned()],
            version: 3,
        };
        let view = content_view(AgeState::DeclaredAdult, &settings);

        assert_eq!(view.effective_max_rating, "mature");
        assert_eq!(view.version, 3);
        assert_eq!(view.excluded_warnings, vec!["character death".to_owned()]);
    }
}
