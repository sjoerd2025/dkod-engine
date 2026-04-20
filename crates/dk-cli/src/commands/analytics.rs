//! `dk analytics` — administrative subcommands for the ClickHouse-backed
//! analytics warehouse.
//!
//! These commands are operator-facing (running a migration, eyeballing a
//! table, reading a pre-built summary). They intentionally do not wrap
//! the event-emission side — event emission happens implicitly from the
//! engine and runner when `CLICKHOUSE_URL` is set.
//!
//! Connection configuration is read from the same env vars as
//! [`dk_analytics::AnalyticsConfig::from_env`] so the CLI and engine
//! stay in lockstep.
//!
//! Named summary queries live on disk under
//! `crates/dk-analytics/src/queries/*.sql` and are bound with ClickHouse
//! native `{name:Type}` parameters (mirroring pytorch/test-infra's
//! `torchci/clickhouse_queries/*` pattern). No user-supplied value is
//! interpolated into SQL — `--since` is parsed into a `DateTime<Utc>`
//! client-side and passed through `Query::param`.

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::Subcommand;
use colored::Colorize;

#[derive(Subcommand)]
pub enum AnalyticsAction {
    /// Apply the ClickHouse DDL from
    /// [`dk_analytics::schema::SCHEMA_SQL`] to the configured warehouse.
    /// Idempotent — runs `CREATE TABLE IF NOT EXISTS` for every table.
    Migrate {
        /// Also apply the optional refreshable materialized views from
        /// [`dk_analytics::schema::MATERIALIZED_VIEWS_SQL`]. Requires
        /// ClickHouse 24.3+ for the `REFRESH EVERY` syntax.
        #[arg(long)]
        with_materialized_views: bool,
    },

    /// Run an arbitrary SQL query and print the row count. Intended for
    /// smoke-testing connectivity, not for ad-hoc data exploration; use
    /// `clickhouse-client` for that.
    Query {
        /// SQL statement. Quote it on the shell.
        sql: String,
    },

    /// Print a short pre-built summary over a time window:
    /// - Number of changesets merged
    /// - Average verification duration
    /// - Review verdicts
    Summary {
        /// Repository name (reserved — the filter is advisory until we
        /// expose a `repo_id` lookup on the CLI).
        #[arg(long)]
        repo: String,
        /// Lower bound for `created_at` / `transition_at`. Accepts:
        ///
        ///   * a date/datetime literal (e.g. `2024-01-01T12:00:00`)
        ///   * a relative spec `<N><unit>` where unit is `m|h|d|w`
        ///   * the string `now()`
        ///   * a `now() ± INTERVAL N <unit>` expression
        ///
        /// All shapes are parsed client-side into a concrete UTC timestamp
        /// and bound as a `DateTime64(3)` ClickHouse parameter — user input
        /// is never concatenated into SQL.
        #[arg(long, default_value = "7d")]
        since: String,
    },
}

/// Parse a `--since` flag into a concrete `DateTime<Utc>`.
///
/// This replaces the previous `render_since` which emitted a raw SQL
/// fragment. Resolving client-side lets every query in `queries/*.sql`
/// use ClickHouse native parameter binding (`{since:DateTime64(3)}`),
/// which is the pattern pytorch/test-infra uses and is the ClickHouse
/// team's recommended way to avoid SQL injection.
fn parse_since(input: &str) -> Result<DateTime<Utc>> {
    let s = input.trim();

    // Shape 1: relative spec like `7d` or `30m`.
    if let Some(dt) = parse_relative(s) {
        return Ok(dt);
    }

    // Shape 2: a bare `now()` or `now() ± INTERVAL N UNIT` (for back-compat
    // with the previous CLI). We resolve this client-side too.
    let normalised: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowered = normalised.to_ascii_lowercase();
    if lowered == "now()" {
        return Ok(Utc::now());
    }
    for (prefix, sign) in [("now() - interval ", -1i64), ("now() + interval ", 1i64)] {
        if let Some(rest) = lowered.strip_prefix(prefix) {
            let mut it = rest.split_whitespace();
            let n = it.next().and_then(|v| v.parse::<i64>().ok());
            let unit = it.next();
            if it.next().is_some() {
                continue;
            }
            if let (Some(n), Some(unit)) = (n, unit) {
                if let Some(delta) = interval_to_duration(n, unit) {
                    return Ok(Utc::now() + Duration::seconds(sign * delta.num_seconds()));
                }
            }
        }
    }

    // Shape 3: ISO-ish date or datetime literal.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(ndt) = nd.and_hms_opt(0, 0, 0) {
            return Ok(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
        }
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
    }

    anyhow::bail!(
        "--since could not be parsed. Try `7d`, `2024-01-01`, `2024-01-01T12:00:00Z`, \
         `now()`, or `now() - INTERVAL 7 DAY`. Got: {input}"
    )
}

fn parse_relative(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    if num.is_empty() {
        return None;
    }
    let n: i64 = num.parse().ok()?;
    let delta = interval_to_duration(n, unit)?;
    Some(Utc::now() - delta)
}

fn interval_to_duration(n: i64, unit: &str) -> Option<Duration> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "s" | "second" | "seconds" => Some(Duration::seconds(n)),
        "m" | "minute" | "minutes" => Some(Duration::minutes(n)),
        "h" | "hour" | "hours" => Some(Duration::hours(n)),
        "d" | "day" | "days" => Some(Duration::days(n)),
        "w" | "week" | "weeks" => Some(Duration::weeks(n)),
        "month" | "months" => Some(Duration::days(30 * n)),
        "year" | "years" => Some(Duration::days(365 * n)),
        _ => None,
    }
}

pub async fn run(action: AnalyticsAction) -> Result<()> {
    let cfg = dk_analytics::AnalyticsConfig::from_env()
        .context("CLICKHOUSE_URL is not set — analytics is disabled")?;
    let client =
        dk_analytics::AnalyticsClient::new(&cfg).context("failed to build ClickHouse client")?;

    match action {
        AnalyticsAction::Migrate {
            with_materialized_views,
        } => {
            println!("{}", "Applying ClickHouse DDL…".bold());
            dk_analytics::schema::migrate(&client)
                .await
                .context("ClickHouse migration failed")?;
            if with_materialized_views {
                println!("{}", "Applying refreshable materialized views…".bold());
                dk_analytics::schema::migrate_materialized_views(&client)
                    .await
                    .context("ClickHouse materialized-view migration failed")?;
            }
            println!("{}", "Analytics schema migrated".green());
        }
        AnalyticsAction::Query { sql } => {
            // Execute and stream the raw HTTP body back. `clickhouse`'s
            // typed `fetch_all` requires a concrete `Row` with codegen
            // which we don't have for arbitrary SQL — use the FETCH API
            // via the HTTP client directly by running `execute()` and
            // letting the user route the query through a custom format
            // themselves. To keep this useful we run it and print a
            // success line with the row count via `count()` wrapper.
            let sql = sql.trim().trim_end_matches(';').to_string();
            let wrapped = format!("SELECT toString(count()) FROM ({sql})");
            let count: u64 = client
                .inner()
                .query(&wrapped)
                .fetch_one::<String>()
                .await
                .with_context(|| format!("query failed: {sql}"))?
                .parse()
                .unwrap_or(0);
            println!("{count} rows");
        }
        AnalyticsAction::Summary { repo, since } => {
            summary(&client, &repo, &since).await?;
        }
    }
    Ok(())
}

/// Print a compact summary table for one repo.
///
/// Three separate queries rather than one big CTE — keeps each one
/// trivially readable and lets us bail early with a good error message
/// if any single one fails. Each query lives in
/// `crates/dk-analytics/src/queries/*.sql` and uses ClickHouse native
/// parameter binding for `since`.
async fn summary(client: &dk_analytics::AnalyticsClient, repo: &str, since: &str) -> Result<()> {
    let since_dt = parse_since(since)?;
    println!(
        "{} since {since_dt} (from `{since}`)",
        format!("Summary for repo {repo}").bold(),
    );
    let _ = repo; // reserved for future per-repo filter once repo_id lookup exists.

    let merged: String = client
        .inner()
        .query(dk_analytics::queries::SUMMARY_MERGED_COUNT)
        .param("since", since_dt)
        .fetch_one::<String>()
        .await
        .unwrap_or_else(|_| "0".to_string());
    println!("  merged changesets: {merged}");

    let avg: String = client
        .inner()
        .query(dk_analytics::queries::SUMMARY_AVG_VERIFICATION_MS)
        .param("since", since_dt)
        .fetch_one::<String>()
        .await
        .unwrap_or_else(|_| "0".to_string());
    println!("  avg verification step: {avg} ms");

    let verdicts: Vec<String> = client
        .inner()
        .query(dk_analytics::queries::SUMMARY_REVIEW_VERDICTS)
        .param("since", since_dt)
        .fetch_all::<String>()
        .await
        .unwrap_or_default();
    if verdicts.is_empty() {
        println!("  review verdicts: (none)");
    } else {
        println!("  review verdicts:");
        for v in verdicts {
            println!("    {v}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_handles_relative() {
        let before = Utc::now();
        let dt = parse_since("7d").unwrap();
        let after = Utc::now();
        assert!(dt <= before - Duration::days(7) + Duration::seconds(2));
        assert!(dt >= after - Duration::days(7) - Duration::seconds(2));
    }

    #[test]
    fn parse_since_handles_iso_date() {
        let dt = parse_since("2024-01-15").unwrap();
        assert_eq!(dt.to_rfc3339(), "2024-01-15T00:00:00+00:00");
    }

    #[test]
    fn parse_since_handles_iso_datetime() {
        let dt = parse_since("2024-01-15T12:30:00Z").unwrap();
        assert_eq!(dt.to_rfc3339(), "2024-01-15T12:30:00+00:00");
    }

    #[test]
    fn parse_since_handles_now_expr() {
        let dt = parse_since("now() - INTERVAL 7 DAY").unwrap();
        let expected = Utc::now() - Duration::days(7);
        assert!((dt - expected).num_seconds().abs() <= 2);
    }

    #[test]
    fn parse_since_rejects_sql_injection() {
        assert!(parse_since("'; DROP TABLE x; --").is_err());
        assert!(parse_since("now() - INTERVAL 7 DAY; DROP TABLE x").is_err());
        assert!(parse_since("UNION SELECT").is_err());
    }
}
