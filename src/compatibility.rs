//! Conservative D1 policy checks, separate from observed local SQLite outcomes.
use crate::report::{Report, Severity};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;
pub fn check(c: &Connection, r: &mut Report) -> Result<()> {
    let enabled: bool = c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    r.checks.insert(
        "d1_foreign_key_policy".into(),
        if enabled {
            "pass: enforcement enabled; deferral allowed"
        } else {
            "fail: enforcement disabled"
        }
        .into(),
    );
    r.checks.insert(
        "d1_runtime".into(),
        "unverified: local SQLite approximation only".into(),
    );
    if !enabled {
        r.finding(
            "MG001",
            Severity::Critical,
            "D1 foreign key policy not enforced",
            "database",
            None,
            json!({"foreign_keys":false}),
            "Enable foreign keys; use defer_foreign_keys for temporary relationship changes.",
        );
    }
    Ok(())
}
