use crate::{
    report::{
        Report,
        Severity::{Critical, High, Medium},
    },
    schema::{Schema, normalized, quote},
};
use anyhow::Result;
use rusqlite::Connection;
use serde_json::json;
use sqlparser::{
    ast::{AlterTableOperation, FromTable, RenameTableNameKind, Statement, TableFactor},
    dialect::SQLiteDialect,
    parser::Parser,
};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Syntax {
    pub tables: BTreeMap<String, String>,
    pub columns: BTreeMap<(String, String), String>,
    pub explicit_transactions: bool,
}
fn table_name(t: &sqlparser::ast::TableWithJoins) -> Option<String> {
    if let TableFactor::Table { name, .. } = &t.relation {
        Some(name.to_string().trim_matches('"').to_string())
    } else {
        None
    }
}
pub fn syntax(sql: &str, r: &mut Report, file: &str, before: &Schema) -> Syntax {
    let mut out = Syntax::default();
    let statements = match Parser::parse_sql(&SQLiteDialect {}, sql) {
        Ok(s) => s,
        Err(_) => {
            r.limitations.push(format!("{file}: static SQL parsing unavailable; mutation/rename/transaction analysis incomplete. SQLite execution remains authoritative."));
            return out;
        }
    };
    for s in statements {
        let unbounded = match &s {
            Statement::Update {
                table,
                selection: None,
                ..
            } => table_name(table),
            Statement::Delete(d) if d.selection.is_none() => {
                let (FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t)) = &d.from;
                t.first().and_then(table_name)
            }
            _ => None,
        };
        if let Some(t) = unbounded {
            r.finding("SQ001",High,"Potentially destructive mutation without WHERE",&t,Some(file),json!({"baseline_rows":before.tables.get(&t).map(|t|t.rows),"observed":"no WHERE clause","impact":"all matching rows may change; intent is unknown"}),"Review the intended scope; add a predicate or document the full-table operation.");
        }
        match s {
            Statement::StartTransaction { .. }
            | Statement::Commit { .. }
            | Statement::Rollback { .. }
            | Statement::Savepoint { .. }
            | Statement::ReleaseSavepoint { .. } => out.explicit_transactions = true,
            Statement::AlterTable {
                name, operations, ..
            } => {
                let table = name.to_string().trim_matches('"').to_owned();
                for op in operations {
                    match op {
                        AlterTableOperation::RenameColumn {
                            old_column_name,
                            new_column_name,
                        } => {
                            out.columns.insert(
                                (table.clone(), old_column_name.value),
                                new_column_name.value,
                            );
                        }
                        AlterTableOperation::RenameTable {
                            table_name: RenameTableNameKind::To(n) | RenameTableNameKind::As(n),
                        } => {
                            out.tables
                                .insert(table.clone(), n.to_string().trim_matches('"').to_owned());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}
pub fn diff(a: &Schema, b: &Schema, r: &mut Report, file: &str, renames: &Syntax) {
    for (name, t) in &a.tables {
        let target = renames.tables.get(name).unwrap_or(name);
        if target != name && b.tables.contains_key(target) {
            r.schema_changes
                .push(format!("Renamed table {name} to {target} (explicit SQL)"));
        }
        let Some(u) = b.tables.get(target) else {
            r.schema_changes.push(format!("Removed table {name}"));
            r.finding("DP001",Critical,"Table removed",name,Some(file),json!({"before_rows":t.rows,"after":"absent","impact":"application references break; supplied rows are lost"}),"Retain the table until consumers migrate; verify a recovery plan.");
            continue;
        };
        for col in &t.columns {
            let key = (name.clone(), col.name.clone());
            let dest = renames.columns.get(&key).unwrap_or(&col.name);
            let object = format!("{name}.{}", col.name);
            let Some(new) = u.columns.iter().find(|c| &c.name == dest) else {
                r.schema_changes.push(format!("Removed column {object}"));
                r.finding(
                    "DP002",
                    Critical,
                    "Column removed",
                    &object,
                    Some(file),
                    json!({"before":col,"after":"absent","baseline_rows":t.rows}),
                    "Migrate application readers and preserve required values before removal.",
                );
                continue;
            };
            if dest != &col.name {
                r.schema_changes
                    .push(format!("Renamed column {object} to {dest} (explicit SQL)"));
            }
            if col.declared_type.to_uppercase() != new.declared_type.to_uppercase()
                || col.hidden != new.hidden
                || col.not_null != new.not_null
            {
                r.schema_changes.push(format!("Changed column {object}"));
                r.finding("SC001",High,"Column definition changed",&object,Some(file),json!({"before":col,"after":new}),"Review type, generated expression and nullability compatibility with every writer.");
            }
            if !col.not_null && new.not_null {
                r.finding(
                    "SC002",
                    High,
                    "NOT NULL requirement introduced",
                    &object,
                    Some(file),
                    json!({"before":false,"after":true,"tested_rows":t.rows}),
                    "Backfill NULL values and update writers before requiring a value.",
                );
            }
            if col.default.as_deref().map(normalized) != new.default.as_deref().map(normalized) {
                r.finding(
                    "SC004",
                    Medium,
                    "Default value changed",
                    &object,
                    Some(file),
                    json!({"before":col.default,"after":new.default}),
                    "Verify omitted-column inserts still have the intended behavior.",
                );
            }
        }
        for col in &u.columns {
            if !t.columns.iter().any(|c| c.name == col.name)
                && !renames.columns.values().any(|n| n == &col.name)
            {
                r.schema_changes
                    .push(format!("Added column {name}.{}", col.name));
                if col.not_null {
                    r.finding(
                        "SC002",
                        High,
                        "New required column",
                        &format!("{name}.{}", col.name),
                        Some(file),
                        json!({"after":col}),
                        "Verify all writers supply a value or a suitable default.",
                    );
                }
            }
        }
        let pk = |t: &crate::schema::Table| {
            let mut p = t
                .columns
                .iter()
                .filter(|c| c.pk > 0)
                .map(|c| (c.pk, c.name.clone()))
                .collect::<Vec<_>>();
            p.sort();
            p
        };
        let oldpk: Vec<_> = pk(t)
            .into_iter()
            .map(|(i, n)| {
                (
                    i,
                    renames
                        .columns
                        .get(&(name.clone(), n.clone()))
                        .cloned()
                        .unwrap_or(n),
                )
            })
            .collect();
        if oldpk != pk(u) {
            r.finding(
                "SC003",
                High,
                "Primary key changed",
                name,
                Some(file),
                json!({"before":oldpk,"after":pk(u)}),
                "Review identity stability and all referencing relationships.",
            );
        }
        for (ix, v) in &t.indexes {
            let same = u.indexes.get(ix).is_some_and(|n| {
                n.unique == v.unique
                    && n.terms == v.terms
                    && n.sql.as_deref().map(normalized) == v.sql.as_deref().map(normalized)
            });
            if !same {
                r.schema_changes
                    .push(format!("Removed or changed index {ix}"));
                r.finding("IX001",Medium,"Index removed or changed",ix,Some(file),json!({"before":v,"after":u.indexes.get(ix)}),"Compare representative workload plans and retain an equivalent index where needed.");
            }
        }
        for (ix, v) in &u.indexes {
            if !t.indexes.contains_key(ix) {
                r.schema_changes.push(format!("Added index {ix}"));
            }
            if v.terms.iter().any(|t| t.0 == -2) {
                r.limitations.push(format!(
                    "Expression index {ix}: expression semantics compared as SQL tokens only."
                ));
            }
        }
        for (ix, v) in t.indexes.iter().filter(|(_, i)| i.unique) {
            if !u.indexes.values().any(|n| {
                n.unique
                    && n.terms == v.terms
                    && n.partial == v.partial
                    && (!v.partial
                        || n.sql.as_deref().map(normalized) == v.sql.as_deref().map(normalized))
            }) {
                r.finding("SC005",High,"Unique guarantee removed or changed",ix,Some(file),json!({"before":v,"after":"no equivalent unique index"}),"Restore equivalent uniqueness before accepting writes that could create duplicates.");
            }
        }
        if t.foreign_keys != u.foreign_keys {
            r.schema_changes
                .push(format!("Changed foreign keys {name}"));
            if !t.foreign_keys.is_empty() {
                r.finding(
                    "FK002",
                    High,
                    "Foreign key definition removed or changed",
                    name,
                    Some(file),
                    json!({"before":t.foreign_keys,"after":u.foreign_keys}),
                    "Review relationship enforcement and ON DELETE / ON UPDATE behavior.",
                );
            }
        }
        if t.strict != u.strict || t.without_rowid != u.without_rowid {
            r.finding("SC001",High,"Table storage or strictness changed",name,Some(file),json!({"before_strict":t.strict,"after_strict":u.strict,"before_without_rowid":t.without_rowid,"after_without_rowid":u.without_rowid}),"Verify type enforcement and row identity expectations.");
        }
        if normalized(&t.sql) != normalized(&u.sql) {
            r.schema_changes
                .push(format!("Changed table definition {name}"));
            r.limitations.push(format!("{name}: CHECK, collation, generated-expression and deferrability semantics require manual review of changed table SQL."));
        }
    }
    for name in b.tables.keys() {
        if !a.tables.contains_key(name) && !renames.tables.values().any(|n| n == name) {
            r.schema_changes.push(format!("Added table {name}"));
        }
    }
    for (name, (kind, sql)) in &a.objects {
        if !b
            .objects
            .get(name)
            .is_some_and(|(_, s)| normalized(s) == normalized(sql))
        {
            r.schema_changes
                .push(format!("Removed or changed {kind} {name}"));
            r.finding(
                "OB001",
                Medium,
                "View or trigger removed or changed",
                name,
                Some(file),
                json!({"before":sql,"after":b.objects.get(name)}),
                "Review application dependencies and trigger side effects.",
            );
        }
    }
    for (name, (kind, _)) in &b.objects {
        if !a.objects.contains_key(name) {
            r.schema_changes.push(format!("Added {kind} {name}"));
        }
    }
}
/// Compare key survival without serializing keys or customer values.
pub fn rows(
    a: &Connection,
    b: &Connection,
    sa: &Schema,
    sb: &Schema,
    r: &mut Report,
    file: &str,
    renames: &Syntax,
) -> Result<()> {
    for (name, t) in &sa.tables {
        let dest = renames.tables.get(name).unwrap_or(name);
        let Some(u) = sb.tables.get(dest) else {
            continue;
        };
        if u.rows < t.rows {
            r.finding(
                "DP003",
                Critical,
                "Existing rows lost",
                name,
                Some(file),
                json!({"before_count":t.rows,"after_count":u.rows,"minimum_lost":t.rows-u.rows}),
                "Verify the copy predicate and restore omitted records.",
            );
            continue;
        }
        let mut pk = t.columns.iter().filter(|c| c.pk > 0).collect::<Vec<_>>();
        pk.sort_by_key(|c| c.pk);
        if pk.is_empty()
            || t.rows > 100_000
            || pk
                .iter()
                .any(|c| !c.not_null && c.declared_type.to_uppercase() != "INTEGER")
        {
            if t.rows > 0 {
                r.limitations.push(format!("{name}: row identity survival unverified (no reliable bounded primary key); equal counts do not prove preservation."));
            }
            continue;
        }
        let old = pk
            .iter()
            .map(|c| quote(&c.name))
            .collect::<Vec<_>>()
            .join(",");
        let new = pk
            .iter()
            .map(|c| {
                renames
                    .columns
                    .get(&(name.clone(), c.name.clone()))
                    .unwrap_or(&c.name)
            })
            .collect::<Vec<_>>();
        if new.iter().any(|n| !u.columns.iter().any(|c| &c.name == *n)) {
            continue;
        }
        fn keys(c: &Connection, sql: &str) -> Result<Option<std::collections::HashSet<Vec<u8>>>> {
            use rusqlite::types::ValueRef;
            let mut s = c.prepare(sql)?;
            let n = s.column_count();
            let mut rows = s.query([])?;
            let mut keys = std::collections::HashSet::new();
            let mut bytes = 0usize;
            while let Some(row) = rows.next()? {
                let mut key = vec![];
                for i in 0..n {
                    let value = row.get_ref(i)?;
                    let (tag, data): (u8, Vec<u8>) = match value {
                        ValueRef::Null => return Ok(None),
                        ValueRef::Integer(v) => (1, v.to_le_bytes().to_vec()),
                        ValueRef::Real(v) => (2, v.to_bits().to_le_bytes().to_vec()),
                        ValueRef::Text(v) if v.len() <= 65_536 => (3, v.to_vec()),
                        ValueRef::Blob(v) if v.len() <= 65_536 => (4, v.to_vec()),
                        _ => return Ok(None),
                    };
                    key.push(tag);
                    key.extend_from_slice(&(data.len() as u64).to_le_bytes());
                    key.extend_from_slice(&data);
                    if key.len() > 65_536 {
                        return Ok(None);
                    }
                }
                bytes += key.len();
                if bytes > 32 * 1024 * 1024 || keys.len() >= 100_000 {
                    return Ok(None);
                }
                keys.insert(key);
            }
            Ok(Some(keys))
        }
        let ka = keys(a, &format!("SELECT {old} FROM {}", quote(name)))?;
        let kb = keys(
            b,
            &format!(
                "SELECT {} FROM {}",
                new.iter().map(|n| quote(n)).collect::<Vec<_>>().join(","),
                quote(dest)
            ),
        )?;
        let (Some(ka), Some(kb)) = (ka, kb) else {
            r.limitations.push(format!(
                "{name}: primary-key survival unverified due to NULL keys or row/key memory bounds."
            ));
            continue;
        };
        let lost = ka.difference(&kb).count();
        if lost > 0 {
            r.finding("DP003",Critical,"Baseline primary key identities disappeared",name,Some(file),json!({"missing_primary_keys":lost,"interpretation":"records removed or re-keyed; values are not emitted"}),"Verify whether re-keying is intentional; otherwise restore missing records.");
        }
    }
    Ok(())
}
pub fn constraints(
    c: &Connection,
    r: &mut Report,
    file: Option<&str>,
    fk: bool,
    integrity: bool,
) -> Result<()> {
    if fk {
        match c.prepare("PRAGMA foreign_key_check") {
            Ok(mut s) => {
                let violations=s.query_map([],|row|Ok(json!({"child_table":row.get::<_,String>(0)?,"row_identifier":row.get::<_,Option<i64>>(1)?,"parent_table":row.get::<_,String>(2)?,"constraint_id":row.get::<_,i64>(3)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
                r.checks.insert(
                    "foreign_keys".into(),
                    if violations.is_empty() {
                        "pass"
                    } else {
                        "fail"
                    }
                    .into(),
                );
                for v in violations {
                    r.finding(
                        "FK001",
                        Critical,
                        "Foreign key integrity violation",
                        "database",
                        file,
                        v,
                        "Repair orphaned relationships before deployment.",
                    );
                }
            }
            Err(_) => {
                r.checks.insert(
                    "foreign_keys".into(),
                    "failed to validate relationship metadata".into(),
                );
                r.finding(
                    "FK001",
                    Critical,
                    "Foreign key check could not validate schema",
                    "database",
                    file,
                    json!({"check":"foreign_key_check could not prepare"}),
                    "Correct invalid relationship definitions.",
                );
            }
        }
    } else {
        r.checks
            .insert("foreign_keys".into(), "skipped by configuration".into());
    }
    if integrity {
        let statuses = c
            .prepare("PRAGMA integrity_check")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let ok = statuses == ["ok"];
        r.checks
            .insert("integrity".into(), if ok { "pass" } else { "fail" }.into());
        if !ok {
            r.finding("MG001",Critical,"Database integrity check failed","database",file,json!({"check":"integrity_check","failure_count":statuses.len(),"details":"omitted to avoid exposing row values"}),"Investigate the candidate database and baseline integrity.");
        }
    } else {
        r.checks
            .insert("integrity".into(), "skipped by configuration".into());
    }
    Ok(())
}
pub fn plan(c: &Connection, sql: &str) -> rusqlite::Result<Vec<crate::report::PlanStep>> {
    let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
    let params = vec![rusqlite::types::Null; s.parameter_count()];
    s.query_map(rusqlite::params_from_iter(params), |r| {
        Ok(crate::report::PlanStep {
            id: r.get(0)?,
            parent: r.get(1)?,
            detail: r.get(3)?,
        })
    })?
    .collect()
}
pub fn queries(
    a: &Connection,
    b: &Connection,
    path: &std::path::Path,
    r: &mut Report,
    file: Option<&str>,
) -> Result<()> {
    for p in crate::engine::files(path)? {
        let sql = crate::engine::read_sql(&p)?;
        let statements = match Parser::parse_sql(&SQLiteDialect {}, &sql) {
            Ok(v) => v,
            Err(_) => {
                r.limitations.push(format!(
                    "{}: workload skipped; static parser could not establish statement boundaries",
                    p.display()
                ));
                continue;
            }
        };
        for (i, s) in statements.iter().enumerate() {
            let label = format!("{}#{}", p.display(), i + 1);
            if !matches!(s, Statement::Query(_)) {
                r.limitations
                    .push(format!("{label}: non-SELECT workload skipped"));
                continue;
            }
            let before = plan(a, &s.to_string());
            let after = plan(b, &s.to_string());
            let be = before.is_err();
            let ae = after.is_err();
            let before_steps = before.unwrap_or_default();
            let after_steps = after.unwrap_or_default();
            let before: Vec<String> = before_steps.iter().map(|p| p.detail.clone()).collect();
            let after: Vec<String> = after_steps.iter().map(|p| p.detail.clone()).collect();
            if !be && ae {
                r.finding("QP002",Critical,"Previously valid workload query fails",&label,file,json!({"before":"prepared successfully","after":"prepare failed (details omitted for privacy)"}),"Update the query or preserve its schema dependencies.");
            }
            let search_to_scan = after
                .iter()
                .filter(|s| s.starts_with("SCAN ") && !s.contains("USING"))
                .any(|new| {
                    let table = new.split_whitespace().nth(1);
                    before.iter().any(|old| {
                        old.starts_with("SEARCH ")
                            && old.contains("USING")
                            && old.split_whitespace().nth(1) == table
                    })
                });
            let scan = after
                .iter()
                .any(|s| s.starts_with("SCAN ") && !s.contains("USING"));
            let new_temp = after.iter().any(|s| s.contains("TEMP B-TREE"))
                && !before.iter().any(|s| s.contains("TEMP B-TREE"));
            let new_auto = after.iter().any(|s| s.contains("AUTOMATIC"))
                && !before.iter().any(|s| s.contains("AUTOMATIC"));
            if !be && !ae && (search_to_scan || new_temp || new_auto) {
                r.finding("IX002",High,"Observed query plan regression",&label,file,json!({"before":before,"after":after,"impact":"potential additional work; no latency measurement"}),"Review representative data and restore or replace the supporting index.");
            } else if !ae && scan {
                r.finding("QP001",Medium,"Potential expensive table scan",&label,file,json!({"before":before,"after":after,"impact":"scan may be appropriate for this workload"}),"Review expected table size and selectivity before adding an index.");
            }
            if be {
                r.limitations.push(format!(
                    "{label}: baseline query invalid; no regression conclusion available."
                ));
            }
            r.queries.push(crate::report::Query {
                file: label,
                before,
                after,
                before_steps,
                after_steps,
                baseline_error: be,
                candidate_error: ae,
            });
        }
    }
    Ok(())
}
