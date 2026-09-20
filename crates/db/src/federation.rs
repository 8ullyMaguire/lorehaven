//! ActivityPub federation, instance fingerprints, and similarity-based peer selection.
//!
//! Ported from gravity's federation/fingerprint services, adapted for lorehaven:
//! - ApActor, ApActivity, ApFollow tables
//! - Instance fingerprints (theme_vector + cultural + content signals)
//! - Multi-dimensional similarity (Jaccard fallback, cosine-ready)
//! - Auto-federation with hysteresis/grace period

use crate::{Backend, Database};
use anyhow::Result;
use serde_json::Value as JsonValue;
use std::collections::HashSet;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApActor {
    pub id: String,
    pub actor_type: String,
    pub user_id: Option<String>,
    pub instance_host: Option<String>,
    pub ap_id: String,
    pub inbox_url: String,
    pub outbox_url: String,
    pub followers_url: Option<String>,
    pub following_url: Option<String>,
    pub public_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApActivity {
    pub id: String,
    pub activity_type: String,
    pub actor_id: String,
    pub object_id: Option<String>,
    pub object_type: Option<String>,
    pub payload: JsonValue,
    pub created_at: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApFollow {
    pub id: String,
    pub follower_actor_id: String,
    pub followed_actor_id: String,
    pub accepted: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InstanceFingerprint {
    pub id: String,
    pub instance_host: String,
    pub fingerprint_version: i64,
    pub fingerprint_json: JsonValue,
    pub theme_vector: Option<JsonValue>,
    pub cultural_signals: JsonValue,
    pub content_signals: JsonValue,
    pub signature: String,
    pub valid_until: String,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FederationPeer {
    pub id: String,
    pub peer_host: String,
    pub similarity: f64,
    pub state: String,
    pub last_checked: Option<String>,
    pub auto_federate: bool,
    pub set_by: Option<String>,
    pub set_at: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SimilarityBreakdown {
    pub theme: f64,
    pub cultural: f64,
    pub content: f64,
    pub overall: f64,
}

// ---------------------------------------------------------------------------
// AP Actors
// ---------------------------------------------------------------------------

pub async fn create_actor(
    db: &Database,
    actor_type: &str,
    user_id: Option<&str>,
    instance_host: Option<&str>,
    ap_id: &str,
    inbox_url: &str,
    outbox_url: &str,
    followers_url: Option<&str>,
    following_url: Option<&str>,
    public_key: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO ap_actors (id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&id).bind(actor_type).bind(user_id).bind(instance_host)
                .bind(ap_id).bind(inbox_url).bind(outbox_url).bind(followers_url).bind(following_url)
                .bind(public_key).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO ap_actors (id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at) VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)")
                .bind(&id).bind(actor_type).bind(user_id).bind(instance_host)
                .bind(ap_id).bind(inbox_url).bind(outbox_url).bind(followers_url).bind(following_url)
                .bind(public_key).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn get_actor_by_ap_id(db: &Database, ap_id: &str) -> Result<Option<ApActor>> {
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String, String, Option<String>, Option<String>, String, String)>(
                "SELECT id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at FROM ap_actors WHERE ap_id = ?"
            ).bind(ap_id).fetch_optional(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, Option<String>, Option<String>, String, String, String, Option<String>, Option<String>, String, String)>(
                "SELECT id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at FROM ap_actors WHERE ap_id = $1"
            ).bind(ap_id).fetch_optional(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(row.map(|(id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at)| {
        ApActor { id, actor_type, user_id, instance_host, ap_id, inbox_url, outbox_url, followers_url, following_url, public_key, created_at }
    }))
}

// ---------------------------------------------------------------------------
// AP Activities
// ---------------------------------------------------------------------------

pub async fn create_activity(
    db: &Database,
    activity_type: &str,
    actor_id: &str,
    object_id: Option<&str>,
    object_type: Option<&str>,
    payload: &JsonValue,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    let payload_str = serde_json::to_string(payload)?;
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO ap_activities (id, activity_type, actor_id, object_id, object_type, payload, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
                .bind(&id).bind(activity_type).bind(actor_id).bind(object_id).bind(object_type)
                .bind(&payload_str).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO ap_activities (id, activity_type, actor_id, object_id, object_type, payload, created_at) VALUES ($1::uuid, $2, $3, $4, $5, $6::jsonb, $7)")
                .bind(&id).bind(activity_type).bind(actor_id).bind(object_id).bind(object_type)
                .bind(&payload_str).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

// ---------------------------------------------------------------------------
// AP Follows
// ---------------------------------------------------------------------------

pub async fn create_follow(db: &Database, follower_id: &str, followed_id: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO ap_follows (id, follower_actor_id, followed_actor_id, accepted, created_at) VALUES (?, ?, ?, FALSE, ?)")
                .bind(&id).bind(follower_id).bind(followed_id).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO ap_follows (id, follower_actor_id, followed_actor_id, accepted, created_at) VALUES ($1::uuid, $2::uuid, $3::uuid, FALSE, $4)")
                .bind(&id).bind(follower_id).bind(followed_id).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn accept_follow(db: &Database, follow_id: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE ap_follows SET accepted = TRUE WHERE id = ?")
                .bind(follow_id).execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE ap_follows SET accepted = TRUE WHERE id = $1::uuid")
                .bind(follow_id).execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------

/// Generate a fingerprint for this instance from its theme vector and content signals.
pub async fn regenerate_fingerprint(
    db: &Database,
    instance_host: &str,
    theme_vector: &JsonValue,
    cultural_signals: &JsonValue,
    content_signals: &JsonValue,
) -> Result<JsonValue> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    // Valid for 30 days
    let valid_until = crate::identity::in_seconds(30 * 86400);
    let signature = format!("sig_{}", instance_host);

    let fp_json = serde_json::json!({
        "instance_host": instance_host,
        "fingerprint_version": 1,
        "generated_at": now,
        "valid_until": valid_until,
        "theme_vector": theme_vector,
        "cultural_signals": cultural_signals,
        "content_signals": content_signals
    });
    let fp_str = serde_json::to_string(&fp_json)?;
    let tv_str = serde_json::to_string(theme_vector)?;
    let cs_str = serde_json::to_string(cultural_signals)?;
    let ct_str = serde_json::to_string(content_signals)?;

    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO instance_fingerprints (id, instance_host, fingerprint_version, fingerprint_json, theme_vector, cultural_signals, content_signals, signature, valid_until, created_at) VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?)")
                .bind(&id).bind(instance_host).bind(&fp_str).bind(&tv_str).bind(&cs_str).bind(&ct_str)
                .bind(&signature).bind(&valid_until).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO instance_fingerprints (id, instance_host, fingerprint_version, fingerprint_json, theme_vector, cultural_signals, content_signals, signature, valid_until, created_at) VALUES ($1::uuid, $2, 1, $3::jsonb, $4::jsonb, $5::jsonb, $6::jsonb, $7, $8, $9)")
                .bind(&id).bind(instance_host).bind(&fp_str).bind(&tv_str).bind(&cs_str).bind(&ct_str)
                .bind(&signature).bind(&valid_until).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(fp_json)
}

/// Get the current valid fingerprint for an instance.
pub async fn get_fingerprint(db: &Database, instance_host: &str) -> Result<Option<InstanceFingerprint>> {
    let now = crate::identity::now_rfc3339();
    let row = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, i64, String, Option<String>, String, String, String, String, String)>(
                "SELECT id, instance_host, fingerprint_version, fingerprint_json, theme_vector, cultural_signals, content_signals, signature, valid_until, created_at FROM instance_fingerprints WHERE instance_host = ? AND valid_until > ? ORDER BY fingerprint_version DESC LIMIT 1"
            ).bind(instance_host).bind(&now).fetch_optional(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, i64, String, Option<String>, String, String, String, String, String)>(
                "SELECT id, instance_host, fingerprint_version, fingerprint_json, theme_vector, cultural_signals, content_signals, signature, valid_until, created_at FROM instance_fingerprints WHERE instance_host = $1 AND valid_until > $2 ORDER BY fingerprint_version DESC LIMIT 1"
            ).bind(instance_host).bind(&now).fetch_optional(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(row.map(|(id, instance_host, fingerprint_version, fp_json, tv, cs, ct, signature, valid_until, created_at)| {
        InstanceFingerprint {
            id, instance_host, fingerprint_version,
            fingerprint_json: serde_json::from_str(&fp_json).unwrap_or_default(),
            theme_vector: tv.and_then(|s| serde_json::from_str(&s).ok()),
            cultural_signals: serde_json::from_str(&cs).unwrap_or_default(),
            content_signals: serde_json::from_str(&ct).unwrap_or_default(),
            signature, valid_until, created_at,
        }
    }))
}

// ---------------------------------------------------------------------------
// Similarity computation
// ---------------------------------------------------------------------------

/// Extract tag set from a fingerprint's theme_vector.
fn extract_theme_tags(fp: &InstanceFingerprint) -> HashSet<String> {
    let mut tags = HashSet::new();
    if let Some(tv) = &fp.theme_vector {
        if let Some(obj) = tv.as_object() {
            for k in obj.keys() {
                tags.insert(k.clone());
            }
        }
    }
    tags
}

/// Jaccard overlap similarity on theme tags (0.0-1.0).
fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() { return 0.0; }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 { 0.0 } else { intersection / union }
}

/// Cultural similarity (0.0-1.0).
fn cultural_similarity(a: &JsonValue, b: &JsonValue) -> f64 {
    let keys = ["adult_content_policy", "languages"];
    let mut matches = 0;
    let mut total = 0;
    for key in keys {
        total += 1;
        if a.get(key) == b.get(key) { matches += 1; }
    }
    if total == 0 { 0.5 } else { matches as f64 / total as f64 }
}

/// Content similarity (0.0-1.0).
fn content_similarity(a: &JsonValue, b: &JsonValue) -> f64 {
    let keys = ["media_ratio", "avg_post_length"];
    let mut sum_diff = 0.0;
    let mut count = 0;
    for key in keys {
        if let (Some(lv), Some(rv)) = (a.get(key).and_then(|v| v.as_f64()), b.get(key).and_then(|v| v.as_f64())) {
            sum_diff += (lv - rv).abs();
            count += 1;
        }
    }
    if count == 0 { 0.5 } else { 1.0 - (sum_diff / count as f64) }
}

/// Compute multi-dimensional similarity between two fingerprints (0.0-100.0).
pub fn compute_similarity(local: &InstanceFingerprint, remote: &InstanceFingerprint) -> SimilarityBreakdown {
    let local_tags = extract_theme_tags(local);
    let remote_tags = extract_theme_tags(remote);
    let theme = jaccard_similarity(&local_tags, &remote_tags);
    let cultural = cultural_similarity(&local.cultural_signals, &remote.cultural_signals);
    let content = content_similarity(&local.content_signals, &remote.content_signals);
    // Weights: 0.60 theme, 0.25 cultural, 0.15 content
    let overall = (0.60 * theme + 0.25 * cultural + 0.15 * content) * 100.0;
    SimilarityBreakdown { theme, cultural, content, overall }
}

// ---------------------------------------------------------------------------
// Federation peers
// ---------------------------------------------------------------------------

/// Upsert a federation peer with similarity score.
pub async fn upsert_peer(
    db: &Database,
    peer_host: &str,
    similarity: f64,
    state: &str,
    auto_federate: bool,
    set_by: Option<&str>,
) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO federation_peers_v2 (id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(peer_host) DO UPDATE SET similarity = excluded.similarity, state = excluded.state, last_checked = excluded.last_checked, auto_federate = excluded.auto_federate, set_by = excluded.set_by, set_at = excluded.set_at")
                .bind(&Uuid::new_v4().to_string()).bind(peer_host).bind(similarity).bind(state)
                .bind(&now).bind(auto_federate).bind(set_by).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO federation_peers_v2 (id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at) VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (peer_host) DO UPDATE SET similarity = EXCLUDED.similarity, state = EXCLUDED.state, last_checked = EXCLUDED.last_checked, auto_federate = EXCLUDED.auto_federate, set_by = EXCLUDED.set_by, set_at = EXCLUDED.set_at")
                .bind(&Uuid::new_v4().to_string()).bind(peer_host).bind(similarity).bind(state)
                .bind(&now).bind(auto_federate).bind(set_by).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

/// List peers above a similarity threshold, ordered by similarity desc.
pub async fn list_similar_peers(db: &Database, threshold: f64, limit: i64) -> Result<Vec<FederationPeer>> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, f64, String, Option<String>, bool, Option<String>, Option<String>)>(
                "SELECT id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at FROM federation_peers_v2 WHERE similarity >= ? ORDER BY similarity DESC LIMIT ?"
            ).bind(threshold).bind(limit).fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, f64, String, Option<String>, bool, Option<String>, Option<String>)>(
                "SELECT id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at FROM federation_peers_v2 WHERE similarity >= $1 ORDER BY similarity DESC LIMIT $2"
            ).bind(threshold).bind(limit).fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(rows.into_iter().map(|(id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at)| {
        FederationPeer { id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at }
    }).collect())
}

/// List all peers.
pub async fn list_all_peers(db: &Database) -> Result<Vec<FederationPeer>> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, f64, String, Option<String>, bool, Option<String>, Option<String>)>(
                "SELECT id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at FROM federation_peers_v2 ORDER BY similarity DESC"
            ).fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, f64, String, Option<String>, bool, Option<String>, Option<String>)>(
                "SELECT id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at FROM federation_peers_v2 ORDER BY similarity DESC"
            ).fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(rows.into_iter().map(|(id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at)| {
        FederationPeer { id, peer_host, similarity, state, last_checked, auto_federate, set_by, set_at }
    }).collect())
}

// ---------------------------------------------------------------------------
// Federation queue
// ---------------------------------------------------------------------------

pub async fn enqueue_activity(db: &Database, activity_id: &str, target_inbox: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("INSERT INTO federation_queue (id, activity_id, target_inbox, attempts, status, created_at) VALUES (?, ?, ?, 0, 'pending', ?)")
                .bind(&id).bind(activity_id).bind(target_inbox).bind(&now)
                .execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("INSERT INTO federation_queue (id, activity_id, target_inbox, attempts, status, created_at) VALUES ($1::uuid, $2::uuid, $3, 0, 'pending', $4)")
                .bind(&id).bind(activity_id).bind(target_inbox).bind(&now)
                .execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(id)
}

pub async fn get_pending_queue(db: &Database, limit: i64) -> Result<Vec<(String, String, String)>> {
    let rows = match db.backend() {
        Backend::Sqlite => {
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT id, activity_id, target_inbox FROM federation_queue WHERE status = 'pending' ORDER BY created_at LIMIT ?"
            ).bind(limit).fetch_all(db.sqlite_pool().expect("sqlite")).await?
        }
        Backend::Postgres => {
            sqlx::query_as::<_, (String, String, String)>(
                "SELECT id, activity_id, target_inbox FROM federation_queue WHERE status = 'pending' ORDER BY created_at LIMIT $1"
            ).bind(limit).fetch_all(db.postgres_pool().expect("postgres")).await?
        }
    };
    Ok(rows)
}

pub async fn mark_queue_sent(db: &Database, id: &str) -> Result<()> {
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE federation_queue SET status = 'sent', processed_at = ? WHERE id = ?")
                .bind(&now).bind(id).execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE federation_queue SET status = 'sent', processed_at = $1 WHERE id = $2::uuid")
                .bind(&now).bind(id).execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}

pub async fn mark_queue_failed(db: &Database, id: &str) -> Result<()> {
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query("UPDATE federation_queue SET status = 'failed', attempts = attempts + 1 WHERE id = ?")
                .bind(id).execute(db.sqlite_pool().expect("sqlite")).await?;
        }
        Backend::Postgres => {
            sqlx::query("UPDATE federation_queue SET status = 'failed', attempts = attempts + 1 WHERE id = $1::uuid")
                .bind(id).execute(db.postgres_pool().expect("postgres")).await?;
        }
    }
    Ok(())
}
