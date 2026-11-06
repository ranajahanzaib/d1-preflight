use crate::{
    config::{Config, Options},
    schema::quote,
};
use anyhow::{Context, Result, bail};
use rusqlite::{
    Connection, OpenFlags,
    backup::Backup,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub struct Sandbox {
    pub baseline: Connection,
    pub candidate: Connection,
    _temp: tempfile::TempDir,
}
pub fn guard(c: &mut Connection) -> Result<()> {
    guard_mode(c, false)
}
pub fn migration_guard(c: &mut Connection) -> Result<()> {
    guard_mode(c, true)
}
fn guard_mode(c: &mut Connection, migration: bool) -> Result<()> {
    c.busy_timeout(Duration::from_secs(3))?;
    c.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
    c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA temp_store=MEMORY; PRAGMA max_page_count=262144;")?;
    c.set_limit(Limit::SQLITE_LIMIT_LENGTH, 16 * 1024 * 1024)?;
    c.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 4 * 1024 * 1024)?;
    c.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    let start = Instant::now();
    c.progress_handler(
        10_000,
        Some(move || start.elapsed() > Duration::from_secs(30)),
    )?;
    c.authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
        AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } if migration => {
            Authorization::Deny
        }
        AuthAction::Attach { .. }
        | AuthAction::Detach { .. }
        | AuthAction::CreateVtable { .. }
        | AuthAction::DropVtable { .. } => Authorization::Deny,
        AuthAction::Function { function_name }
            if ["load_extension", "writefile", "readfile", "fts3_tokenizer"]
                .iter()
                .any(|s| function_name.eq_ignore_ascii_case(s)) =>
        {
            Authorization::Deny
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value,
        } if pragma_value.is_some()
            && ![
                "defer_foreign_keys",
                "foreign_keys",
                "table_info",
                "table_xinfo",
                "index_list",
                "index_xinfo",
                "foreign_key_list",
            ]
            .iter()
            .any(|s| pragma_name.eq_ignore_ascii_case(s)) =>
        {
            Authorization::Deny
        }
        AuthAction::Pragma {
            pragma_name,
            pragma_value: Some(v),
        } if pragma_name.eq_ignore_ascii_case("foreign_keys")
            && !["1", "on", "true"]
                .iter()
                .any(|s| v.eq_ignore_ascii_case(s)) =>
        {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))?;
    Ok(())
}
pub fn backup(from: &Connection, to: &mut Connection) -> Result<()> {
    let start = Instant::now();
    let b = Backup::new(from, to)?;
    loop {
        match b.step(128)? {
            rusqlite::backup::StepResult::Done => break,
            _ if start.elapsed() > Duration::from_secs(30) => {
                bail!("snapshot exceeded 30 second resource bound")
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    Ok(())
}
impl Sandbox {
    pub fn new(o: &Options) -> Result<Self> {
        let temp = tempfile::tempdir()?;
        let mut baseline = Connection::open(temp.path().join("baseline.sqlite"))?;
        if let Some(p) = &o.db {
            let source = Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| crate::error::input("cannot open source SQLite database read-only"))?;
            source.busy_timeout(Duration::from_secs(3))?;
            backup(&source, &mut baseline).map_err(|_|crate::error::input("cannot snapshot source SQLite database; verify integrity, access, WAL state and contention"))?;
            guard(&mut baseline)?;
        } else {
            guard(&mut baseline)?;
            let p = o.schema.as_ref().context("missing baseline")?;
            baseline
                .execute_batch(&read_sql(p)?)
                .map_err(|_|crate::error::input("baseline schema SQL was rejected; verify SQL syntax and sandbox-compatible PRAGMAs"))?;
            if let Some(p) = &o.data {
                baseline
                    .execute_batch(&read_sql(p)?)
                    .map_err(|_|crate::error::input("baseline data SQL was rejected; verify schema and constraint compatibility"))?;
            }
            if !baseline.is_autocommit() {
                return Err(crate::error::input(
                    "baseline SQL leaves an open transaction; add COMMIT",
                ));
            }
        }
        let mut candidate = Connection::open(temp.path().join("candidate.sqlite"))?;
        backup(&baseline, &mut candidate)?;
        guard(&mut candidate)?;
        Ok(Self {
            baseline,
            candidate,
            _temp: temp,
        })
    }
}
pub fn read_sql(p: &Path) -> Result<String> {
    if std::fs::metadata(p).map_err(crate::error::input)?.len() > 4 * 1024 * 1024 {
        return Err(crate::error::input(format!(
            "SQL file exceeds 4 MiB bound: {}",
            p.display()
        )));
    }
    std::fs::read_to_string(p)
        .with_context(|| format!("reading SQL {}", p.display()))
        .map_err(crate::error::input)
}
pub fn files(path: &Path) -> Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        bail!("not a file or directory: {}", path.display());
    }
    let mut result = vec![];
    for e in std::fs::read_dir(path)? {
        let p = e?.path();
        if p.is_dir() {
            if std::fs::symlink_metadata(&p)?.file_type().is_symlink() {
                bail!("symlinked directories are not traversed: {}", p.display());
            }
            result.extend(files(&p)?);
        } else if p.extension().is_some_and(|s| s == "sql") {
            result.push(p);
        }
    }
    result.sort();
    Ok(result)
}
pub fn pending(o: &Options, c: &Config, db: &Connection) -> Result<Vec<(String, PathBuf)>> {
    let mut selected = vec![];
    if let Some(dir) = &o.migrations_dir {
        let discovered = if let Some(pattern) = &c.migrations.pattern {
            glob::glob(pattern)?.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            files(dir)?
        };
        let mut all = discovered
            .into_iter()
            .map(|p| {
                let id = p
                    .strip_prefix(dir)
                    .with_context(|| {
                        format!("migration {} is outside {}", p.display(), dir.display())
                    })?
                    .to_string_lossy()
                    .replace('\\', "/");
                Ok((id, p))
            })
            .collect::<Result<Vec<_>>>()?;
        all.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some(after) = &c.migrations.after {
            if !all.iter().any(|(n, _)| n == after) {
                bail!("migration boundary '{after}' not found; use its exact relative path");
            }
            all.retain(|(n, _)| n > after);
        } else if o.db.is_some() {
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [&c.migrations.tracking_table],
                |r| r.get(0),
            )?;
            if !exists {
                bail!(
                    "no migration tracking table; select --migration explicitly or set migrations.after"
                );
            }
            let cols = db
                .prepare("SELECT name FROM pragma_table_info(?1)")?
                .query_map([&c.migrations.tracking_table], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let name=cols.iter().find(|s|s.eq_ignore_ascii_case("name")).context("tracking table has no recognized name column; use explicit --migration or migrations.after")?;
            let applied = db
                .prepare(&format!(
                    "SELECT {} FROM {}",
                    quote(name),
                    quote(&c.migrations.tracking_table)
                ))?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
            for name in &applied {
                if !all.iter().any(|(id, _)| id == name)
                    && all.iter().any(|(id, _)| {
                        id.contains('/') && id.rsplit('/').next() == Some(name.as_str())
                    })
                {
                    bail!(
                        "ambiguous legacy migration identity '{name}' matches a nested path; use explicit pending files or migrations.after"
                    );
                }
            }
            all.retain(|(n, _)| !applied.contains(n));
        } else {
            bail!(
                "schema plus migration directory is ambiguous; use --migration or configure migrations.after"
            );
        }
        selected.extend(all);
    }
    selected.extend(
        o.migration
            .iter()
            .map(|p| (p.to_string_lossy().into_owned(), p.clone())),
    );
    if selected.is_empty() && o.migrations_dir.is_none() {
        bail!("select --migration or --migrations-dir");
    }
    selected.sort_by(|a, b| a.0.cmp(&b.0));
    let mut seen = std::collections::BTreeSet::new();
    for (_, p) in &selected {
        if !seen.insert(p.canonicalize()?) {
            bail!("duplicate migration: {}", p.display());
        }
        read_sql(p)?;
    }
    Ok(selected)
}
