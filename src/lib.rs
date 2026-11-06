pub mod analysis;
pub mod compatibility;
pub mod config;
pub mod engine;
pub mod error;
pub mod report;
pub mod schema;
use anyhow::{Context, Result};
use config::{Config, Options};
use report::{Migration, Report, Severity};
use serde_json::json;
pub fn analyze(o: &Options, c: &Config) -> Result<Report> {
    let mut db = engine::Sandbox::new(o)?;
    let migrations = engine::pending(o, c, &db.baseline).map_err(error::input)?;
    let baseline = schema::snapshot(&db.baseline, &c.migrations.tracking_table).map_err(|_| {
        error::input("baseline schema could not be introspected; check SQLite compatibility")
    })?;
    let mut current = baseline.clone();
    let mut r = Report::new(
        if o.db.is_some() {
            "sqlite_snapshot"
        } else {
            "schema_sql"
        }
        .into(),
    );
    if baseline.tables.values().all(|t| t.rows == 0) {
        r.limitations.push("Baseline has no application rows; data-dependent safety checks have limited confidence.".into());
    }
    let mut stopped = false;
    for (order, (file, path)) in migrations.iter().enumerate() {
        if stopped {
            r.migrations.push(Migration {
                file: file.clone(),
                order,
                status: "skipped".into(),
                error: None,
            });
            continue;
        }
        let sql = engine::read_sql(path).map_err(error::input)?;
        let syntax = analysis::syntax(&sql, &mut r, file, &current);
        let previous_file = tempfile::NamedTempFile::new()?;
        let mut previous = rusqlite::Connection::open(previous_file.path())?;
        engine::backup(&db.candidate, &mut previous)?;
        engine::guard(&mut previous)?;
        // Reject explicit transactions conservatively: nesting would diverge from D1.
        let result = if syntax.explicit_transactions {
            Err(anyhow::anyhow!(
                "explicit transaction control unsupported; remove BEGIN/COMMIT/SAVEPOINT for D1-style per-file atomic execution"
            ))
        } else {
            (|| -> Result<()> {
                engine::guard(&mut db.candidate)?;
                db.candidate.execute_batch("BEGIN IMMEDIATE")?;
                engine::migration_guard(&mut db.candidate)?;
                let execution = db.candidate.execute_batch(&sql);
                engine::guard(&mut db.candidate)?;
                execution?;
                if db.candidate.is_autocommit() {
                    anyhow::bail!("migration escaped the enclosing transaction");
                }
                analysis::constraints(
                    &db.candidate,
                    &mut r,
                    Some(file),
                    c.analysis.check_foreign_keys,
                    false,
                )?;
                db.candidate.execute_batch("COMMIT")?;
                Ok(())
            })()
        };
        if let Err(error) = result {
            let sqlite_error = error
                .downcast_ref::<rusqlite::Error>()
                .and_then(|e| e.sqlite_error())
                .map(|e| json!({"code":format!("{:?}",e.code),"extended_code":e.extended_code}));
            let _ = db.candidate.execute_batch("ROLLBACK");
            // SQLite diagnostics can contain literals or private uniqueness values.
            let message = if syntax.explicit_transactions {
                "Explicit transaction control unsupported; use implicit per-file transactions."
            } else {
                "SQLite rejected migration or commit; inspect SQL, constraints, sandbox policy, and resource bounds. Raw diagnostics omitted for privacy."
            };
            r.migrations.push(Migration {
                file: file.clone(),
                order,
                status: "failed".into(),
                error: Some(message.into()),
            });
            r.finding(
                "MG001",
                Severity::Critical,
                "Migration execution failed",
                file,
                Some(file),
                json!({"error":message,"sqlite_error":sqlite_error,"candidate":"rolled back; dependent migrations skipped"}),
                "Correct the failing SQL and rerun against representative data.",
            );
            r.status = "migration_failed".into();
            stopped = true;
        } else {
            let next = schema::snapshot(&db.candidate, &c.migrations.tracking_table)?;
            analysis::diff(&current, &next, &mut r, file, &syntax);
            // A snapshot of each prior state is required for per-migration row matching.
            analysis::rows(
                &previous,
                &db.candidate,
                &current,
                &next,
                &mut r,
                file,
                &syntax,
            )?;
            current = next;
            r.migrations.push(Migration {
                file: file.clone(),
                order,
                status: "completed".into(),
                error: None,
            });
        }
    }
    analysis::constraints(
        &db.candidate,
        &mut r,
        None,
        c.analysis.check_foreign_keys,
        c.analysis.check_integrity,
    )?;
    if let Some(p) = &o.queries {
        if c.analysis.check_query_plans {
            analysis::queries(&db.baseline, &db.candidate, p, &mut r, None)?;
        } else {
            r.limitations
                .push("Query plans skipped by configuration.".into());
        }
    } else {
        r.limitations
            .push("No workload supplied; query compatibility and plan coverage absent.".into());
    }
    r.checks.insert(
        "migration_execution".into(),
        if stopped { "fail" } else { "pass" }.into(),
    );
    compatibility::check(&db.candidate, &mut r)?;
    r.schema_changes.sort();
    r.schema_changes.dedup();
    r.limitations.sort();
    r.limitations.dedup();
    r.finish(&c.rules);
    Ok(r)
}
pub fn write_output(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}
