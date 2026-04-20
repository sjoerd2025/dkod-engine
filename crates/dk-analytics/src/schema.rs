//! DDL + naive migrator. Reads [`SCHEMA_SQL`] and runs each statement in
//! order against the configured ClickHouse client.
//!
//! Two DDL bundles are shipped:
//!
//! | Const | Applied by | Purpose |
//! |-------|-----------|---------|
//! | [`SCHEMA_SQL`] | `dk analytics migrate` | Base MergeTree tables for raw events |
//! | [`MATERIALIZED_VIEWS_SQL`] | `dk analytics migrate --with-materialized-views` | Optional `REFRESH EVERY` MVs for dashboards (ClickHouse 24.3+) |

use anyhow::{Context, Result};

use crate::client::AnalyticsClient;

/// Base ClickHouse schema — kept inline so `dk analytics migrate` does not
/// depend on the binary's working directory.
pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Optional refreshable materialized views, following pytorch/test-infra's
/// pattern. Requires ClickHouse 24.3+ for the `REFRESH EVERY ...` clause,
/// which is why it is applied separately from the base schema.
pub const MATERIALIZED_VIEWS_SQL: &str = include_str!("materialized_views.sql");

/// Parse a DDL bundle into a sequence of statements separated by `;`,
/// skipping SQL line comments. Exposed for tests and for operators who want
/// to preview what `migrate` will execute.
pub fn statements(schema: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in schema.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("--") || trimmed.is_empty() {
            continue;
        }
        buf.push_str(line);
        buf.push('\n');
        if line.trim_end().ends_with(';') {
            let stmt = buf.trim_end().trim_end_matches(';').trim().to_string();
            if !stmt.is_empty() {
                out.push(stmt);
            }
            buf.clear();
        }
    }
    let tail = buf.trim().trim_end_matches(';').trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Apply a DDL bundle statement by statement, attaching the offending SQL
/// to any error for easier operator debugging.
async fn apply_bundle(client: &AnalyticsClient, bundle: &str, label: &str) -> Result<()> {
    for stmt in statements(bundle) {
        tracing::debug!(target: "dk_analytics", bundle = label, "applying DDL: {stmt}");
        client
            .inner()
            .query(&stmt)
            .execute()
            .await
            .with_context(|| format!("applying {label} DDL statement: {stmt}"))?;
    }
    Ok(())
}

/// Run the base DDL against ClickHouse. Idempotent — every statement uses
/// `CREATE TABLE IF NOT EXISTS`.
pub async fn migrate(client: &AnalyticsClient) -> Result<()> {
    apply_bundle(client, SCHEMA_SQL, "schema").await
}

/// Run the optional materialized-view DDL. Idempotent. Fails on ClickHouse
/// older than 24.3 because the `REFRESH EVERY` syntax is unsupported there.
pub async fn migrate_materialized_views(client: &AnalyticsClient) -> Result<()> {
    apply_bundle(client, MATERIALIZED_VIEWS_SQL, "materialized_views").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_parses_schema_sql_into_four_tables() {
        let stmts = statements(SCHEMA_SQL);
        assert_eq!(stmts.len(), 4, "expected 4 DDL statements, got {stmts:?}");
        for table in [
            "session_events",
            "changeset_lifecycle",
            "verification_runs",
            "review_results",
        ] {
            assert!(
                stmts.iter().any(|s| s.contains(table)),
                "missing CREATE TABLE for {table}"
            );
        }
    }

    #[test]
    fn statements_strips_comments_and_trailing_semicolons() {
        let src = "-- leading comment\nCREATE TABLE x (a UInt8) ENGINE = Memory;\n-- trailing\nCREATE TABLE y (b UInt8) ENGINE = Memory";
        let stmts = statements(src);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE x"));
        assert!(stmts[1].starts_with("CREATE TABLE y"));
        assert!(!stmts[0].ends_with(';'));
    }

    #[test]
    fn materialized_views_bundle_has_two_tables_and_two_views() {
        let stmts = statements(MATERIALIZED_VIEWS_SQL);
        let tables: Vec<_> = stmts
            .iter()
            .filter(|s| s.trim_start().starts_with("CREATE TABLE"))
            .collect();
        let views: Vec<_> = stmts
            .iter()
            .filter(|s| s.trim_start().starts_with("CREATE MATERIALIZED VIEW"))
            .collect();
        assert_eq!(tables.len(), 2, "expected 2 target tables");
        assert_eq!(views.len(), 2, "expected 2 refreshable views");
        assert!(views.iter().all(|s| s.contains("REFRESH EVERY")));
    }

    #[test]
    fn schema_columns_declare_comments() {
        assert!(
            SCHEMA_SQL.contains("COMMENT 'dkod changeset id'"),
            "expected inline column COMMENT docs"
        );
    }
}
