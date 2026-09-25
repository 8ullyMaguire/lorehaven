//! Discovery routes: recommendations, taste profiles.
//!
//! Spec §16.1–16.2.

use crate::auth::{MaybeSession, RequireSession};
use crate::http::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use lorehaven_db;
use lorehaven_domain::ids::WorkId;
use lorehaven_domain::AppError;
use std::str::FromStr;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discovery", get(get_discovery))
        .route("/discovery/taste-profile", get(get_taste_profile))
        .route("/discovery/taste-profile/me", get(get_my_taste_vector))
        .route(
            "/discovery/taste-profile/recompute",
            post(recompute_taste_profile),
        )
        .route("/discovery/taste-profile/clear", post(clear_taste_profile))
        .route(
            "/operator/taste-profile",
            get(get_admin_taste_profile).put(update_admin_taste_profile),
        )
        .route(
            "/operator/taste-profile/recompute-all",
            post(recompute_all_taste_profiles),
        )
        .route("/me/streak", get(get_my_streak))
        .route("/media/reverse-search", post(reverse_search))
        .route("/curator/bounty-queue", get(curator_bounty_queue))
        .route("/operator/affinities", post(set_operator_affinity))
        .nest("/recipes", recipe_routes())
        .nest("/dashboard", dashboard_routes())
}

/// `GET /me/streak` — the signed-in user's streak state (spec §9.7.1).
async fn get_my_streak(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let db = state.db();
    let row: Option<(i64, i64, Option<String>, i64)> =
        match db.backend() {
            lorehaven_db::Backend::Sqlite => sqlx::query_as(
                "SELECT current_streak, longest_streak, last_login_at, streak_freezes_used
                 FROM streaks WHERE account_id = ?",
            )
            .bind(user.account_id.to_string())
            .fetch_optional(db.sqlite_pool().ok_or(ApiError(AppError::Internal(
                anyhow::anyhow!("db pool unavailable"),
            )))?)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?,
            lorehaven_db::Backend::Postgres => sqlx::query_as(
                "SELECT current_streak, longest_streak, last_login_at, streak_freezes_used
                 FROM streaks WHERE account_id = $1::uuid",
            )
            .bind(user.account_id.to_string())
            .fetch_optional(db.postgres_pool().ok_or(ApiError(AppError::Internal(
                anyhow::anyhow!("db pool unavailable"),
            )))?)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?,
        };
    let (current, longest, last_login_at, freezes_used) = row.unwrap_or((0, 0, None, 0));
    Ok(Json(serde_json::json!({
        "current_streak": current,
        "longest_streak": longest,
        "last_login_at": last_login_at,
        "streak_freezes_used": freezes_used,
    })))
}

fn recipe_routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_recipe))
        .route("/{id}", get(get_recipe_route).post(update_recipe_route))
        .route("/{id}/delete", post(delete_recipe_route))
        .route("/list", get(list_recipes_route))
}

fn dashboard_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_dashboard))
        .route("/", post(save_dashboard))
}

/// Query params for `GET /discovery` (spec §43.2).
#[derive(Debug, serde::Deserialize)]
pub struct DiscoveryQuery {
    #[serde(default)]
    sort: Option<String>,
}

async fn get_discovery(
    State(state): State<AppState>,
    MaybeSession(session): MaybeSession,
    Query(params): Query<DiscoveryQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let limit = 20;
    let account_id: Option<String> = session.as_ref().map(|s| s.account_id.to_string());

    // Resolve the effective sort (spec §43.4): query param > stored preference > default.
    let effective_sort =
        resolve_sort(&state, session.as_ref(), params.sort.as_deref(), "discover").await;

    // Build candidate lists from each recommendation engine, then blend.
    let mut engines: Vec<Vec<lorehaven_domain::discovery::Candidate>> = Vec::new();

    if let Some(ref account_id) = account_id {
        // Engine 1: tag-based personalization (from taste profile).
        let personalized =
            lorehaven_db::discovery::personalized_recommendations(state.db(), account_id, limit)
                .await
                .map_err(|e| ApiError(AppError::Internal(e)))?;
        engines.push(
            personalized
                .into_iter()
                .enumerate()
                .map(|(idx, id)| lorehaven_domain::discovery::Candidate {
                    work_id: id,
                    score: (limit - idx as i64),
                    reason: "tags".into(),
                    taste_signal: 0.0,
                    diversity_class: 0.5,
                })
                .collect(),
        );
    }

    // Engine 2: popularity-based (public engine).
    let popular = lorehaven_db::discovery::public_recommendations(state.db(), limit)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    engines.push(
        popular
            .into_iter()
            .enumerate()
            .map(|(idx, id)| lorehaven_domain::discovery::Candidate {
                work_id: id,
                score: (limit - idx as i64),
                reason: "popular".into(),
                taste_signal: 0.0,
                diversity_class: 0.5,
            })
            .collect(),
    );

    // Engine 3: media-reference collaborative (signed-in users only).
    // Finds works sharing media references (faceclaims, moodboards, playlists)
    // with the user's bookmarked works. Spec §32.7.3, §9.10.
    if let Some(ref account_id) = account_id {
        let media_collab = lorehaven_db::discovery::media_reference_collaborative_recommendations(
            state.db(),
            account_id,
            limit,
        )
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
        engines.push(
            media_collab
                .into_iter()
                .enumerate()
                .map(|(idx, id)| lorehaven_domain::discovery::Candidate {
                    work_id: id,
                    score: (limit - idx as i64),
                    reason: "media_ref_collab".into(),
                    taste_signal: 0.0,
                    diversity_class: 0.5,
                })
                .collect(),
        );
    }

    // Merge candidates. Branch on rec_mode (spec §16.1a, M52-07):
    // - legacy: blend multi-engine candidates (current behavior)
    // - pluggable: use strategy registry with RRF blend
    let mut blended: Vec<lorehaven_domain::discovery::Candidate> =
        if state.config().discovery.rec_mode == "pluggable" {
            if let Some(ref account_id) = account_id {
                // M52-09: the reader's stored engine preference narrows the
                // blend (spec §16.1b). The resolver is the only thing that
                // reads the preference, so a reader's choice applies here the
                // same as it applies on every other surface — and when the
                // operator has since disabled the reader's choice, the
                // instance blend is used and `choice` says so, rather than a
                // different engine being substituted without notice.
                let choice = crate::rec_preference::load_for_pseud(
                    state.db(),
                    &state.config().discovery,
                    session
                        .as_ref()
                        .and_then(|s| s.pseud_id.as_ref().map(|p| p.as_uuid())),
                )
                .await;
                let registry = crate::rec_engine::build_registry(&state.config().discovery);
                let registry = choice.effective_registry(&registry).unwrap_or_else(|| {
                    crate::rec_engine::build_registry(&state.config().discovery)
                });
                let ids = crate::rec_engine::generate_with_registry(
                    state.db(),
                    &registry,
                    account_id,
                    limit as usize,
                )
                .await
                .map_err(|e| ApiError(AppError::Internal(e)))?;
                ids.into_iter()
                    .map(|id| lorehaven_domain::ids::WorkId::from_str(&id))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| ApiError(AppError::Internal(e.into())))?
                    .into_iter()
                    .enumerate()
                    .map(|(idx, work_id)| lorehaven_domain::discovery::Candidate {
                        work_id,
                        score: (limit - idx as i64),
                        reason: "strategy".into(),
                        taste_signal: 0.0,
                        diversity_class: 0.5,
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            let blended = lorehaven_domain::discovery::blend(&engines);
            blended.into_iter().take(limit as usize).collect()
        };

    // Apply half-life ranking (silent reordering, spec §41.1).
    if state.config().discovery.enable_half_life {
        use lorehaven_db::longevity::half_life_map;
        let ids: Vec<_> = blended.iter().map(|c| c.work_id).collect();
        let half_life_scores = half_life_map(state.db(), &ids)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?;
        let half_life_of = |id: &lorehaven_domain::ids::WorkId| -> Option<i64> {
            half_life_scores
                .iter()
                .find(|(wid, _)| wid == &id.to_string())
                .and_then(|(_, bp)| *bp)
        };
        lorehaven_domain::longevity::apply_half_life(&mut blended, &half_life_of);
    }

    // Apply operator affinity ranking (silent reordering, no field changes).
    let affinities = lorehaven_db::discovery::list_operator_affinities(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    let affinity_map: std::collections::HashMap<String, i64> = affinities
        .into_iter()
        .map(|a| (a.work_id, a.affinity_bp))
        .collect();

    let ranked = if !affinity_map.is_empty() {
        lorehaven_domain::discovery::apply_affinity_ranking(blended, &affinity_map)
    } else {
        // Sort by descending score when no affinities are set.
        let mut r: Vec<lorehaven_domain::discovery::Candidate> = blended.clone();
        r.sort_by_key(|c| -c.score);
        r
    };

    // Apply theme gravity (spec §0.4.6): bounded topic nudge in thematic/adaptive modes.
    let theme = &state.config().theme;
    let ranked = match theme.mode.as_str() {
        "generic" => ranked,
        "thematic" | "adaptive" => {
            let boost: Vec<String> = theme.boost_tags.iter().map(|s| s.to_lowercase()).collect();
            let suppress: Vec<String> = theme
                .suppress_tags
                .iter()
                .map(|s| s.to_lowercase())
                .collect();
            if boost.is_empty() && suppress.is_empty() && theme.tag_gravity_bp.is_empty() {
                ranked
            } else {
                // Pre-fetch tag gravity for all candidates.
                let mut gravity_map: std::collections::HashMap<String, i64> =
                    std::collections::HashMap::new();
                for c in &ranked {
                    let tags = lorehaven_db::taxonomy::tag_names_for_work(
                        state.db(),
                        &c.work_id.to_string(),
                    )
                    .await
                    .map_err(|e| ApiError(AppError::Internal(e)))?
                    .iter()
                    .map(|t| t.to_lowercase())
                    .collect::<Vec<_>>();
                    let mut total_bp: i64 = 0;
                    for tag in &tags {
                        if let Some(&bp) = theme.tag_gravity_bp.get(tag) {
                            total_bp += bp;
                        } else if boost.iter().any(|b| tag.contains(b)) {
                            total_bp += 1000;
                        } else if suppress.iter().any(|s| tag.contains(s)) {
                            total_bp -= 2000;
                        }
                    }
                    gravity_map.insert(c.work_id.to_string(), total_bp);
                }
                let work_nudge =
                    |id: &WorkId| -> i64 { gravity_map.get(&id.to_string()).copied().unwrap_or(0) };
                lorehaven_domain::discovery::apply_theme_gravity(ranked, &work_nudge)
            }
        }
        _ => ranked,
    };

    // Apply taste gravity (spec §9.7.3): weight by taste alignment.
    let taste = &state.config().taste;
    let ranked = lorehaven_domain::discovery::apply_taste_gravity(
        ranked,
        taste.gravity_strength,
        &taste.signal_weight_mode,
        taste.admin_weight,
    );

    let mut items: Vec<serde_json::Value> = ranked
        .into_iter()
        .map(|c| serde_json::json!({ "work_id": c.work_id.to_string() }))
        .collect();

    // Apply diversity: per-fandom caps and exploration slots for signed-in
    // readers (configuration-driven). Anonymous readers pass through as-is.
    if let Some(ref account_id) = account_id {
        if let Some(profile) = lorehaven_db::discovery::taste_profile_for(state.db(), account_id)
            .await
            .map_err(|e| ApiError(AppError::Internal(e)))?
        {
            let fandoms: Vec<String> = match &profile.signals {
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                serde_json::Value::Object(map) => map
                    .get("fandom")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            let known: std::collections::HashSet<String> = fandoms.into_iter().collect();
            let work_ids_ordered: Vec<lorehaven_domain::ids::WorkId> = items
                .iter()
                .filter_map(|item| item["work_id"].as_str().and_then(|s| s.parse().ok()))
                .collect();
            let capped = lorehaven_domain::discovery::apply_diversity(
                work_ids_ordered,
                &known,
                state.config().discovery.per_fandom_cap,
                state.config().discovery.exploration_rate,
                items.len(),
                &|_work_id| Vec::new(), // fandoms fetched per-work in a real impl
            );
            items = capped
                .into_iter()
                .map(|id| serde_json::json!({ "work_id": id.to_string() }))
                .collect();
        }
    }

    // A feed of bare uuids is useless to a human: attach the title and the
    // author's handle so the page can show what each recommendation is.
    let ids: Vec<String> = items
        .iter()
        .filter_map(|item| item["work_id"].as_str().map(|s| s.to_string()))
        .collect();
    let details = lorehaven_db::discovery::work_details_for(state.db(), &ids)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    for item in &mut items {
        if let Some(id) = item["work_id"].as_str() {
            if let Some((title, author)) = details.get(id) {
                item["title"] = serde_json::Value::String(title.clone());
                item["author_handle"] = serde_json::Value::String(author.clone());
            }
        }
    }

    Ok(Json(serde_json::json!({
        "items": items,
        "sort": effective_sort,
    })))
}

/// Resolve the effective sort for a surface (spec §43.4).
///
/// Priority: query param > stored preference > per-surface default.
async fn resolve_sort(
    state: &AppState,
    session: Option<&crate::auth::SessionUser>,
    query_sort: Option<&str>,
    surface: &str,
) -> String {
    // 1. Query param takes precedence.
    if let Some(sort_str) = query_sort {
        if let Some(sort) = lorehaven_domain::browse::Sort::parse(sort_str) {
            return sort.as_str().to_string();
        }
    }

    // 2. Stored preference (if there's a session with a pseud).
    if let Some(user) = session {
        if let Some(pseud_id) = &user.pseud_id {
            if let Ok(pref) = lorehaven_db::browse::get_sort_preference(
                state.db(),
                &pseud_id.to_string(),
                surface,
            )
            .await
            {
                // Clippy suggests collapsing these into a let-chain, which
                // needs edition 2024. This crate is 2021, so the combined
                // Option does the same job without an edition bump.
                if let Some(sort) = pref
                    .as_ref()
                    .and_then(|p| lorehaven_domain::browse::Sort::parse(&p.sort_value))
                {
                    return sort.as_str().to_string();
                }
            }
        }
    }

    // 3. Default for the surface.
    default_for_surface(surface).to_string()
}

fn default_for_surface(surface: &str) -> lorehaven_domain::browse::Sort {
    match surface {
        "discover" => lorehaven_domain::browse::Sort::ForYou,
        "people" | "tags" | "fandoms" | "authors" | "moods" => lorehaven_domain::browse::Sort::Az,
        "library" | "collections" | "series" | "reading-paths" => {
            lorehaven_domain::browse::Sort::New
        }
        _ => lorehaven_domain::browse::Sort::New,
    }
}

async fn get_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    let profile = lorehaven_db::discovery::taste_profile_for(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match profile {
        Some(p) => Ok(Json(serde_json::json!({ "signals": p.signals }))),
        None => Ok(Json(serde_json::json!({ "signals": {} }))),
    }
}

async fn recompute_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    lorehaven_db::discovery::recompute_taste_profile(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "recomputed" })))
}

async fn clear_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    lorehaven_db::discovery::clear_taste_profile(state.db(), &account_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "cleared" })))
}

/// Whether this account may use the operator surface.
pub fn require_operator(state: &AppState, user: &crate::auth::SessionUser) -> ApiResult<()> {
    let configured = state.config().administration.operator_account_id;
    if configured == Some(user.account_id) {
        return Ok(());
    }
    tracing::debug!(
        operator_configured = configured.is_some(),
        "an operator route was reached by an account that is not the operator"
    );
    Err(ApiError(AppError::NotFound { resource: "page" }))
}

/// Get the current user's taste vector.
async fn get_my_taste_vector(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account_id = user.account_id.to_string();
    match lorehaven_db::taste_vectors::get_taste_vector(state.db(), &account_id).await {
        Ok(Some((vec, dist, _))) => Ok(Json(serde_json::json!({
            "dimensions": state.config().taste.dimensions,
            "vector": vec,
            "centroid_distance": dist,
        }))),
        Ok(None) => Ok(Json(serde_json::json!({
            "dimensions": state.config().taste.dimensions,
            "vector": null,
            "centroid_distance": 0.0,
        }))),
        Err(e) => Err(ApiError(AppError::Internal(e.into()))),
    }
}

/// Get the admin taste profile (operator only).
async fn get_admin_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    let dimensions = state.config().taste.dimensions.clone();
    Ok(Json(serde_json::json!({
        "dimensions": dimensions,
        "gravity_strength": state.config().taste.gravity_strength,
        "signal_weight_mode": state.config().taste.signal_weight_mode,
        "admin_weight": state.config().taste.admin_weight,
        "diversity_injection_percent": state.config().taste.diversity_injection_percent,
    })))
}

/// Update the admin taste profile (operator only).
async fn update_admin_taste_profile(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    // TODO: persist updates to config file
    let dimensions = body
        .get("dimensions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| state.config().taste.dimensions.clone());
    Ok(Json(serde_json::json!({
        "status": "updated",
        "dimensions": dimensions,
    })))
}

/// Recompute all taste profiles (operator only).
async fn recompute_all_taste_profiles(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;
    lorehaven_db::taste_vectors::recompute_all_taste_vectors(state.db())
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?;
    Ok(Json(serde_json::json!({ "status": "recompute_started" })))
}

/// Set an operator affinity on a work. Audit-logged; never surfaced publicly.
async fn set_operator_affinity(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    require_operator(&state, &user)?;

    let work_id = body
        .get("work_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("work_id", "work_id is required")))?
        .to_string();
    let affinity_bp = body
        .get("affinity_bp")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ApiError(AppError::field("affinity_bp", "affinity_bp is required")))?;
    let rationale = body
        .get("rationale")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("rationale", "rationale is required")))?
        .to_string();

    let operator = user.account_id.to_string();
    lorehaven_db::discovery::set_operator_affinity(
        state.db(),
        &work_id,
        affinity_bp,
        &operator,
        &rationale,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({ "status": "set" })))
}

// ---------------------------------------------------------------------------
// M11-05: Recipes
// ---------------------------------------------------------------------------

async fn create_recipe(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let recipe_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("id", "id is required")))?
        .to_string();
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("name", "name is required")))?
        .to_string();
    let document = body
        .get("document")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let is_public = body
        .get("is_public")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let owner = user.account_id.to_string();

    lorehaven_db::discovery::save_recipe(
        state.db(),
        &recipe_id,
        &owner,
        &name,
        &document,
        is_public,
        &lorehaven_db::identity::now_rfc3339(),
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e)))?;

    Ok(Json(serde_json::json!({ "id": recipe_id })))
}

async fn get_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer = user.account_id.to_string();
    let recipe = lorehaven_db::discovery::get_recipe(state.db(), &id, &viewer)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match recipe {
        Some(r) => Ok(Json(serde_json::json!({
            "id": r.id,
            "owner": r.owner,
            "name": r.name,
            "document": r.document,
            "is_public": r.is_public,
            "created_at": r.created_at,
        }))),
        None => Err(ApiError(AppError::NotFound { resource: "recipe" })),
    }
}

async fn list_recipes_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let viewer = user.account_id.to_string();
    let recipes = lorehaven_db::discovery::list_recipes(state.db(), &viewer)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "recipes": recipes })))
}

async fn update_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let owner = user.account_id.to_string();
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError(AppError::field("name", "name is required")))?
        .to_string();
    let document = body
        .get("document")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let updated = lorehaven_db::discovery::update_recipe(state.db(), &id, &owner, &name, &document)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;

    if !updated {
        return Err(ApiError(AppError::NotFound { resource: "recipe" }));
    }
    Ok(Json(serde_json::json!({ "status": "updated" })))
}

async fn delete_recipe_route(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let owner = user.account_id.to_string();
    let deleted = lorehaven_db::discovery::delete_recipe(state.db(), &id, &owner)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    if !deleted {
        return Err(ApiError(AppError::NotFound { resource: "recipe" }));
    }
    Ok(Json(serde_json::json!({ "status": "deleted" })))
}

// ---------------------------------------------------------------------------
// M11-06: Dashboards
// ---------------------------------------------------------------------------

async fn get_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let layout = lorehaven_db::discovery::get_dashboard_layout(state.db(), &account)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    match layout {
        Some(l) => Ok(Json(serde_json::json!({ "slots": l.slots }))),
        None => Ok(Json(serde_json::json!({ "slots": [] }))),
    }
}

async fn save_dashboard(
    State(state): State<AppState>,
    RequireSession(user): RequireSession,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = user.account_id.to_string();
    let slots = body.get("slots").cloned().unwrap_or(serde_json::json!([]));
    lorehaven_db::discovery::save_dashboard_layout(state.db(), &account, &slots)
        .await
        .map_err(|e| ApiError(AppError::Internal(e)))?;
    Ok(Json(serde_json::json!({ "status": "saved" })))
}

// ---------------------------------------------------------------------------
// Reverse search & curator bounty queue (spec §32.7.3 & §32.7.5)
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct ReverseSearchPayload {
    pub url: Option<String>,
    pub hash: Option<String>,
    pub algorithm: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReverseSearchView {
    pub references: Vec<serde_json::Value>,
    pub works: Vec<serde_json::Value>,
}

pub async fn reverse_search(
    State(state): State<AppState>,
    _session: MaybeSession,
    Json(payload): Json<ReverseSearchPayload>,
) -> ApiResult<Json<ReverseSearchView>> {
    let hash = if let Some(h) = payload.hash {
        h
    } else if let Some(url) = payload.url {
        let _ = url;
        return Ok(Json(ReverseSearchView {
            references: vec![],
            works: vec![],
        }));
    } else {
        return Err(ApiError(AppError::Validation {
            message: "Either url or hash must be provided".into(),
            field_errors: BTreeMap::new(),
        }));
    };

    // The threshold is the operator's, not a constant in the handler: a
    // self-hosted instance tunes how aggressively it treats two images as the
    // same (spec §32.7.2). The `algorithm` in the request is informational —
    // a hash is only comparable with a hash made the same way, and the
    // comparison below rejects a mismatched pair rather than scoring it.
    let threshold = state.config().media_resilience.perceptual_match_threshold;
    let refs = lorehaven_db::media_resilience::find_by_perceptual_hash(
        state.db(),
        &hash,
        threshold as i32,
    )
    .await
    .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    // The spec asks for a match confidence score the curator can act on, and a
    // bare reference list cannot tell a re-encode of the same image from a
    // different picture that happens to look alike. Distance and score travel
    // together so a client can show both and the operator can set their own bar.
    let refs_json: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| {
            let distance = r.perceptual_hash.as_deref().and_then(|stored| {
                lorehaven_domain::media_resilience::hamming_distance(&hash, stored)
            });
            let exact = r.content_hash == hash;
            serde_json::json!({
                "id": r.id,
                "media_kind": r.media_kind,
                "perceptual_hash": r.perceptual_hash,
                "content_hash": r.content_hash,
                "curator_verified": r.curator_verified,
                "match_kind": if exact { "exact" } else { "perceptual" },
                "match_distance": distance,
                "match_confidence": distance.map(
                    lorehaven_domain::media_resilience::perceptual_match_confidence
                ),
                // The auto-attach decision the spec names, computed once here so
                // the client and the operator agree on it.
                "auto_attach": distance == Some(0),
            })
        })
        .collect();

    // §32.7.3: also return the works that use each reference.
    let mut works_json: Vec<serde_json::Value> = Vec::new();
    for r in &refs {
        let work_refs =
            lorehaven_db::media_resilience::find_works_by_media_reference(state.db(), &r.id)
                .await
                .map_err(|e| ApiError(AppError::Internal(e.into())))?;
        for (work_id, title, display_url) in work_refs {
            works_json.push(serde_json::json!({
                "reference_id": r.id,
                "work_id": work_id,
                "work_title": title,
                "display_url": display_url,
            }));
        }
    }

    Ok(Json(ReverseSearchView {
        references: refs_json,
        works: works_json,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CuratorBountyQueueQuery {
    pub healthy_below: Option<i32>,
    pub limit: Option<i32>,
}

pub async fn curator_bounty_queue(
    State(state): State<AppState>,
    RequireSession(_user): RequireSession,
    Query(query): Query<CuratorBountyQueueQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    let healthy_below = query.healthy_below.unwrap_or(3);
    let limit = query.limit.unwrap_or(50);

    let refs =
        lorehaven_db::media_resilience::find_curator_bounty_queue(state.db(), healthy_below, limit)
            .await
            .map_err(|e| ApiError(AppError::Internal(e.into())))?;

    let refs_json: Vec<serde_json::Value> = refs
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "media_kind": r.media_kind,
                "perceptual_hash": r.perceptual_hash,
                "content_hash": r.content_hash,
                "curator_verified": r.curator_verified,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "queue": refs_json,
        "count": refs_json.len(),
    })))
}
