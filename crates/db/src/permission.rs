//! M27 — Permission statements, derivative lineage, and the exclusion registry.
//!
//! Repository layer for the permission statement on works and accounts, the
//! derivative lineage edge table, and the exclusion registry.
//!
//! Spec §33.1.

use crate::{Backend, Database};
use anyhow::{Context, Result};
pub use lorehaven_domain::permission::{
    ExclusionEntry, ExclusionTarget, LineageEdge, LineageKind, Permission, PermissionStatement,
};
use sqlx::Row;

/// Get a work's permission statement.
pub async fn get_work_permission_statement(
    db: &Database,
    work_id: &str,
) -> Result<PermissionStatement> {
    let sql = db.sql(
        "SELECT permission_statement FROM works WHERE id = ?",
        "SELECT permission_statement FROM works WHERE id = $1::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
                .with_context(|| format!("work not found: {work_id}"))?;
            let stmt_str: String = row.get(0);
            Ok(serde_json::from_str(&stmt_str).unwrap_or_else(|_| PermissionStatement::unstated()))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(work_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
                .with_context(|| format!("work not found: {work_id}"))?;
            let stmt_str: String = row.get(0);
            Ok(serde_json::from_str(&stmt_str).unwrap_or_else(|_| PermissionStatement::unstated()))
        }
    }
}

/// Update a work's permission statement.
pub async fn set_work_permission_statement(
    db: &Database,
    work_id: &str,
    statement: &PermissionStatement,
) -> Result<()> {
    let json =
        serde_json::to_string(&statement).context("failed to serialize permission statement")?;
    let sql = db.sql(
        "UPDATE works SET permission_statement = ?, updated_at = ? WHERE id = ?",
        "UPDATE works SET permission_statement = $1, updated_at = $2 WHERE id = $3::uuid",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(json)
                .bind(&now)
                .bind(work_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(json)
                .bind(&now)
                .bind(work_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Get an account's permission statement.
pub async fn get_account_permission_statement(
    db: &Database,
    account_id: &str,
) -> Result<PermissionStatement> {
    let sql = db.sql(
        "SELECT permission_statement FROM accounts WHERE id = ?",
        "SELECT permission_statement FROM accounts WHERE id = $1::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?
                .with_context(|| format!("account not found: {account_id}"))?;
            let stmt_str: String = row.get(0);
            Ok(serde_json::from_str(&stmt_str).unwrap_or_else(|_| PermissionStatement::unstated()))
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(account_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?
                .with_context(|| format!("account not found: {account_id}"))?;
            let stmt_str: String = row.get(0);
            Ok(serde_json::from_str(&stmt_str).unwrap_or_else(|_| PermissionStatement::unstated()))
        }
    }
}

/// Update an account's permission statement.
pub async fn set_account_permission_statement(
    db: &Database,
    account_id: &str,
    statement: &PermissionStatement,
) -> Result<()> {
    let json =
        serde_json::to_string(&statement).context("failed to serialize permission statement")?;
    let sql = db.sql(
        "UPDATE accounts SET permission_statement = ?, updated_at = ? WHERE id = ?",
        "UPDATE accounts SET permission_statement = $1, updated_at = $2 WHERE id = $3::uuid",
    );
    let now = crate::identity::now_rfc3339();
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(json)
                .bind(&now)
                .bind(account_id)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(json)
                .bind(&now)
                .bind(account_id)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Insert a derivative lineage edge.
pub async fn insert_lineage_edge(db: &Database, edge: &LineageEdge) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO derivative_lineage (id, from_work_id, to_work_id, kind, provenance, created_at) \
              VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO derivative_lineage (id, from_work_id, to_work_id, kind, provenance, created_at) \
              VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&edge.id)
                .bind(&edge.from_work_id)
                .bind(&edge.to_work_id)
                .bind(edge.kind.as_str())
                .bind(&edge.provenance)
                .bind(&edge.created_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&edge.id)
                .bind(&edge.from_work_id)
                .bind(&edge.to_work_id)
                .bind(edge.kind.as_str())
                .bind(&edge.provenance)
                .bind(&edge.created_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Fetch all lineage edges for a work (as parent or child).
pub async fn lineage_edges_for_work(db: &Database, work_id: &str) -> Result<Vec<LineageEdge>> {
    let sql = db.sql(
        "SELECT id, from_work_id, to_work_id, kind, provenance, created_at \
              FROM derivative_lineage \
             WHERE from_work_id = ? OR to_work_id = ? \
          ORDER BY created_at",
        "SELECT id, from_work_id, to_work_id, kind, provenance, created_at \
              FROM derivative_lineage \
             WHERE from_work_id = $1::uuid OR to_work_id = $1::uuid \
          ORDER BY created_at",
    );
    let mut edges = Vec::new();
    match db.backend() {
        Backend::Sqlite => {
            let rows = sqlx::query(&sql)
                .bind(work_id)
                .bind(work_id)
                .fetch_all(db.sqlite_pool().expect("sqlite"))
                .await?;
            for r in rows {
                edges.push(LineageEdge {
                    id: r.get::<String, _>(0),
                    from_work_id: r.get::<String, _>(1),
                    to_work_id: r.get::<String, _>(2),
                    kind: LineageKind::parse(&r.get::<String, _>(3))
                        .unwrap_or(LineageKind::InspiredBy),
                    provenance: r.get::<String, _>(4),
                    created_at: r.get::<String, _>(5),
                });
            }
        }
        Backend::Postgres => {
            let rows = sqlx::query(&sql)
                .bind(work_id)
                .bind(work_id)
                .fetch_all(db.postgres_pool().expect("postgres"))
                .await?;
            for r in rows {
                edges.push(LineageEdge {
                    id: r.get::<String, _>(0),
                    from_work_id: r.get::<String, _>(1),
                    to_work_id: r.get::<String, _>(2),
                    kind: LineageKind::parse(&r.get::<String, _>(3))
                        .unwrap_or(LineageKind::InspiredBy),
                    provenance: r.get::<String, _>(4),
                    created_at: r.get::<String, _>(5),
                });
            }
        }
    }
    Ok(edges)
}

/// Insert an exclusion registry entry.
pub async fn insert_exclusion_entry(db: &Database, entry: &ExclusionEntry) -> Result<()> {
    let sql = db.sql(
        "INSERT INTO exclusion_registry (id, target_type, target_id, reason, created_by, created_at) \
              VALUES (?, ?, ?, ?, ?, ?)",
        "INSERT INTO exclusion_registry (id, target_type, target_id, reason, created_by, created_at) \
              VALUES ($1::uuid, $2, $3::uuid, $4, $5::uuid, $6)",
    );
    match db.backend() {
        Backend::Sqlite => {
            sqlx::query(&sql)
                .bind(&entry.id)
                .bind(entry.target_type.as_str())
                .bind(&entry.target_id)
                .bind(&entry.reason)
                .bind(&entry.created_by)
                .bind(&entry.created_at)
                .execute(db.sqlite_pool().expect("sqlite"))
                .await?;
        }
        Backend::Postgres => {
            sqlx::query(&sql)
                .bind(&entry.id)
                .bind(entry.target_type.as_str())
                .bind(&entry.target_id)
                .bind(&entry.reason)
                .bind(&entry.created_by)
                .bind(&entry.created_at)
                .execute(db.postgres_pool().expect("postgres"))
                .await?;
        }
    }
    Ok(())
}

/// Check if a work or creator is excluded.
pub async fn is_excluded(
    db: &Database,
    target_type: ExclusionTarget,
    target_id: &str,
) -> Result<bool> {
    let sql = db.sql(
        "SELECT id FROM exclusion_registry WHERE target_type = ? AND target_id = ?",
        "SELECT id FROM exclusion_registry WHERE target_type = $1 AND target_id = $2::uuid",
    );
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(&sql)
                .bind(target_type.as_str())
                .bind(target_id)
                .fetch_optional(db.sqlite_pool().expect("sqlite"))
                .await?;
            Ok(row.is_some())
        }
        Backend::Postgres => {
            let row = sqlx::query(&sql)
                .bind(target_type.as_str())
                .bind(target_id)
                .fetch_optional(db.postgres_pool().expect("postgres"))
                .await?;
            Ok(row.is_some())
        }
    }
}

/// Compute the lineage depth of a work — how many ancestors it has in the
/// `remix` chain. A work with no parent returns 0; a work whose parent has
/// no parent returns 1; and so on. Walks up the chain following `remix` edges
/// only (translations and other kinds don't extend the fork chain).
pub async fn lineage_depth(db: &Database, work_id: &str) -> Result<u32> {
    let mut depth = 0u32;
    let mut current = Some(work_id.to_owned());
    // Guard against cycles: walk at most 100 steps regardless of config.
    while let Some(id) = current.take() {
        if depth >= 100 {
            break;
        }
        let sql = db.sql(
            "SELECT from_work_id FROM derivative_lineage WHERE to_work_id = ? AND kind = 'remix' LIMIT 1",
            "SELECT from_work_id FROM derivative_lineage WHERE to_work_id = $1::uuid AND kind = 'remix' LIMIT 1",
        );
        let parent: Option<String> = match db.backend() {
            Backend::Sqlite => {
                sqlx::query_scalar(&sql)
                    .bind(&id)
                    .fetch_optional(db.sqlite_pool().expect("sqlite"))
                    .await?
            }
            Backend::Postgres => {
                sqlx::query_scalar(&sql)
                    .bind(&id)
                    .fetch_optional(db.postgres_pool().expect("postgres"))
                    .await?
            }
        };
        if let Some(p) = parent {
            depth += 1;
            current = Some(p);
        } else {
            break;
        }
    }
    Ok(depth)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorehaven_domain::permission::{LineageKind, Permission};
    use uuid::Uuid;

    /// Create a minimal account → pseud → work chain for testing.
    async fn create_test_work(db: &crate::Database) -> anyhow::Result<String> {
        use crate::content::create_work;
        use crate::identity::AccountStatus;
        use crate::identity::{create_account, create_pseud};
        use lorehaven_domain::policy::AgeState;
        let account = create_account(
            db,
            &format!("test-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .map_err(|e| anyhow::anyhow!("create account: {e}"))?;
        let handle = format!("user-{}", Uuid::new_v4());
        let pseud = create_pseud(db, account, &handle, "Test User")
            .await
            .map_err(|e| anyhow::anyhow!("create pseud: {e}"))?;
        let work = create_work(db, pseud, "Test Work", None)
            .await
            .map_err(|e| anyhow::anyhow!("create work: {e}"))?;
        Ok(work.id.to_string())
    }

    #[tokio::test]
    async fn work_permission_statement_roundtrip() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-perm-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        let work_id = create_test_work(&db).await?;

        // Fetch it (should have default statement)
        let stmt = get_work_permission_statement(&db, &work_id).await?;
        assert!(stmt.is_unstated());

        // Update the statement
        let mut new_stmt = PermissionStatement::unstated();
        new_stmt.podfic = Permission::Yes;
        new_stmt.translation = Permission::No;
        set_work_permission_statement(&db, &work_id, &new_stmt).await?;

        // Fetch again
        let fetched = get_work_permission_statement(&db, &work_id).await?;
        assert_eq!(fetched.podfic, Permission::Yes);
        assert_eq!(fetched.translation, Permission::No);
        assert_eq!(fetched.remix, Permission::Unstated);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn lineage_edge_insert_and_fetch() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-perm-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        // Insert two works
        let work_a = create_test_work(&db).await?;
        let work_b = create_test_work(&db).await?;

        // Insert a lineage edge
        let edge = LineageEdge {
            id: Uuid::new_v4().to_string(),
            from_work_id: work_a.clone(),
            to_work_id: work_b.clone(),
            kind: LineageKind::Translation,
            provenance: "imported from source".to_string(),
            created_at: crate::identity::now_rfc3339(),
        };
        insert_lineage_edge(&db, &edge).await?;

        // Fetch edges for work A
        let edges_a = lineage_edges_for_work(&db, &work_a).await?;
        assert_eq!(edges_a.len(), 1);
        assert_eq!(edges_a[0].from_work_id, work_a);
        assert_eq!(edges_a[0].to_work_id, work_b);
        assert_eq!(edges_a[0].kind, LineageKind::Translation);

        // Fetch edges for work B
        let edges_b = lineage_edges_for_work(&db, &work_b).await?;
        assert_eq!(edges_b.len(), 1);
        assert_eq!(edges_b[0].from_work_id, work_a);
        assert_eq!(edges_b[0].to_work_id, work_b);
        assert_eq!(edges_b[0].kind, LineageKind::Translation);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn exclusion_registry_check() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "lorehaven-db-perm-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        let config =
            crate::DatabaseConfig::new(format!("sqlite://{}/test.db?mode=rwc", dir.display()));
        let db = crate::Database::connect(&config).await.expect("connect");
        db.migrate().await.expect("migrate");

        // Create an account for the exclusion entry's created_by FK
        use crate::identity::{create_account, AccountStatus};
        use lorehaven_domain::policy::AgeState;
        let account = create_account(
            &db,
            &format!("excl-{}@example.test", Uuid::new_v4()),
            AgeState::DeclaredAdult,
            AccountStatus::Active,
        )
        .await
        .map_err(|e| anyhow::anyhow!("create account: {e}"))?;

        // Insert a work to exclude
        let work_id = create_test_work(&db).await?;

        // Initially not excluded
        assert!(!is_excluded(&db, ExclusionTarget::Work, &work_id).await?);

        // Add to exclusion registry
        let entry = ExclusionEntry {
            id: Uuid::new_v4().to_string(),
            target_type: ExclusionTarget::Work,
            target_id: work_id.clone(),
            reason: "test exclusion".to_string(),
            created_by: account.to_string(),
            created_at: crate::identity::now_rfc3339(),
        };
        insert_exclusion_entry(&db, &entry).await?;

        // Now excluded
        assert!(is_excluded(&db, ExclusionTarget::Work, &work_id).await?);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
