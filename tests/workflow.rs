use d1_preflight::{
    analyze,
    config::{Config, Options},
    report::{Report, Severity},
};
use std::path::PathBuf;
fn run(schema: &str, migration: &str, queries: Option<&str>) -> Report {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("schema.sql");
    let m = t.path().join("migration.sql");
    std::fs::write(&s, schema).unwrap();
    std::fs::write(&m, migration).unwrap();
    let q = queries.map(|text| {
        let p = t.path().join("query.sql");
        std::fs::write(&p, text).unwrap();
        p
    });
    analyze(
        &Options {
            schema: Some(s),
            migration: vec![m],
            queries: q,
            ..Default::default()
        },
        &Config::default(),
    )
    .unwrap()
}
fn has(r: &Report, id: &str) -> bool {
    r.findings.iter().any(|f| f.rule_id == id)
}
const USERS: &str = "CREATE TABLE users(id INTEGER PRIMARY KEY,email TEXT,status TEXT DEFAULT 'active'); INSERT INTO users VALUES(1,'PRIVATE_CUSTOMER','active'),(2,'other','active');";
#[test]
fn safe_addition_and_no_false_positives() {
    let r = run(
        USERS,
        "ALTER TABLE users ADD COLUMN last_login TEXT;",
        Some("SELECT email FROM users WHERE id=1;"),
    );
    assert!(!r.failed(Severity::High));
    for id in d1_preflight::config::RULES {
        assert!(!has(&r, id), "unexpected {id}");
    }
    assert!(r.schema_changes.iter().any(|s| s.contains("last_login")));
}
#[test]
fn removed_table_and_column() {
    assert!(has(&run(USERS, "DROP TABLE users;", None), "DP001"));
    let r = run(
        USERS,
        "ALTER TABLE users DROP COLUMN email;",
        Some("SELECT email FROM users"),
    );
    assert!(has(&r, "DP002"));
    assert!(has(&r, "QP002"));
    assert!(!has(&r, "DP001"));
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("PRIVATE_CUSTOMER")
    );
}
#[test]
fn row_loss_on_rebuild() {
    let r = run(
        USERS,
        "CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT,status TEXT DEFAULT 'active');INSERT INTO replacement SELECT * FROM users WHERE id=1;DROP TABLE users;ALTER TABLE replacement RENAME TO users;",
        None,
    );
    assert!(has(&r, "DP003"));
    assert!(!has(&r, "DP001"));
}
#[test]
fn preserving_rebuild() {
    let r = run(
        USERS,
        "CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT,status TEXT DEFAULT 'active');INSERT INTO replacement SELECT * FROM users;DROP TABLE users;ALTER TABLE replacement RENAME TO users;",
        None,
    );
    assert!(!r.failed(Severity::Medium), "{:?}", r.findings);
}
#[test]
fn rekey_without_count_loss() {
    let r = run(USERS, "UPDATE users SET id=id+10 WHERE id>0;", None);
    assert!(has(&r, "DP003"));
}
#[test]
fn no_pk_is_explicitly_uncertain() {
    let r = run(
        "CREATE TABLE t(v TEXT);INSERT INTO t VALUES('secret');",
        "UPDATE t SET v='new' WHERE v='secret';",
        None,
    );
    assert!(!has(&r, "DP003"));
    assert!(
        r.limitations
            .iter()
            .any(|s| s.contains("identity survival unverified"))
    );
}
#[test]
fn explicit_renames() {
    let r = run(
        USERS,
        "ALTER TABLE users RENAME COLUMN email TO address;ALTER TABLE users RENAME TO members;",
        None,
    );
    assert!(!has(&r, "DP001"));
    assert!(!has(&r, "DP002"));
    assert!(!has(&r, "DP003"));
    assert!(r.schema_changes.iter().any(|s| s.contains("Renamed table")));
}
#[test]
fn column_constraints() {
    let r = run(
        "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT DEFAULT 'old');INSERT INTO t VALUES(1,'a');",
        "CREATE TABLE replacement(id TEXT PRIMARY KEY, v BLOB NOT NULL DEFAULT 'new');INSERT INTO replacement SELECT * FROM t;DROP TABLE t;ALTER TABLE replacement RENAME TO t;",
        None,
    );
    for id in ["SC001", "SC002", "SC004"] {
        assert!(has(&r, id), "missing {id}");
    }
}
#[test]
fn primary_key_change() {
    let r = run(
        "CREATE TABLE t(a INTEGER,b INTEGER);",
        "DROP TABLE t; CREATE TABLE t(a INTEGER PRIMARY KEY,b INTEGER);",
        None,
    );
    assert!(has(&r, "SC003"));
}
#[test]
fn uniqueness_removed() {
    let r = run(
        "CREATE TABLE t(id INTEGER PRIMARY KEY,v TEXT);CREATE UNIQUE INDEX ux ON t(v);",
        "DROP INDEX ux;",
        None,
    );
    assert!(has(&r, "SC005"));
    assert!(has(&r, "IX001"));
}
#[test]
fn equivalent_unique_replacement() {
    let r = run(
        "CREATE TABLE t(v TEXT);CREATE UNIQUE INDEX ux ON t(v);",
        "DROP INDEX ux;CREATE UNIQUE INDEX ux_new ON t(v);",
        None,
    );
    assert!(!has(&r, "SC005"));
}
#[test]
fn plan_regression() {
    let r = run(
        "CREATE TABLE orders(id INTEGER PRIMARY KEY,customer_id INTEGER);CREATE INDEX ix ON orders(customer_id);",
        "DROP INDEX ix;",
        Some("SELECT * FROM orders WHERE customer_id=42;"),
    );
    assert!(has(&r, "IX002"));
    assert!(r.queries[0].before.iter().any(|s| s.contains("SEARCH")));
    assert!(r.queries[0].after.iter().any(|s| s.contains("SCAN")));
}
#[test]
fn scan_not_latency_claim() {
    let r = run(
        "CREATE TABLE t(v TEXT);",
        "ALTER TABLE t ADD COLUMN a;",
        Some("SELECT * FROM t;"),
    );
    assert!(has(&r, "QP001"));
    assert!(!has(&r, "IX002"));
}
#[test]
fn workloads_multiple_and_no_execution() {
    let r = run(
        USERS,
        "SELECT 1;",
        Some(
            "SELECT * FROM users WHERE id=?1; DELETE FROM users; SELECT * FROM users WHERE id=:id;",
        ),
    );
    assert_eq!(r.queries.len(), 2);
    assert!(!has(&r, "DP003"));
    assert!(r.limitations.iter().any(|s| s.contains("non-SELECT")));
}
#[test]
fn foreign_keys_failure_and_deferred_success() {
    let s = "CREATE TABLE p(id INTEGER PRIMARY KEY);CREATE TABLE c(id INTEGER PRIMARY KEY,pid INTEGER REFERENCES p(id));INSERT INTO p VALUES(1);INSERT INTO c VALUES(1,1);";
    let r = run(s, "PRAGMA defer_foreign_keys=ON;DELETE FROM p;", None);
    assert!(has(&r, "FK001"));
    assert!(has(&r, "MG001"));
    assert_eq!(r.status, "migration_failed");
    let r = run(
        s,
        "PRAGMA defer_foreign_keys=ON;DELETE FROM p;INSERT INTO p VALUES(1);",
        None,
    );
    assert!(!has(&r, "FK001"));
    assert!(!has(&r, "MG001"));
}
#[test]
fn removed_relationship() {
    let r = run(
        "CREATE TABLE p(id INTEGER PRIMARY KEY);CREATE TABLE c(id INTEGER PRIMARY KEY,pid INTEGER REFERENCES p(id));",
        "DROP TABLE c;CREATE TABLE c(id INTEGER PRIMARY KEY,pid INTEGER);",
        None,
    );
    assert!(has(&r, "FK002"));
}
#[test]
fn object_changes() {
    let r = run(
        "CREATE TABLE t(a);CREATE VIEW v AS SELECT a FROM t;CREATE TRIGGER tr AFTER INSERT ON t BEGIN UPDATE t SET a=a WHERE a IS NULL; END;",
        "DROP VIEW v;DROP TRIGGER tr;",
        None,
    );
    assert_eq!(
        r.findings.iter().filter(|f| f.rule_id == "OB001").count(),
        2
    );
}
#[test]
fn trigger_semicolons_execute() {
    let r = run(
        "CREATE TABLE t(a);CREATE TABLE audit(a);",
        "CREATE TRIGGER tr AFTER INSERT ON t BEGIN INSERT INTO audit VALUES(new.a);INSERT INTO audit VALUES(new.a);END;",
        None,
    );
    assert_eq!(r.status, "completed");
    assert!(!has(&r, "SQ001"));
}
#[test]
fn mutation_structure_handles_comments_and_literals() {
    let r = run(
        USERS,
        "-- WHERE ignored\nUPDATE users SET status='WHERE; -- text';",
        None,
    );
    assert!(has(&r, "SQ001"));
    let r = run(
        USERS,
        "UPDATE users SET status='no WHERE needed' WHERE id IN (SELECT id FROM users WHERE id=1);",
        None,
    );
    assert!(!has(&r, "SQ001"));
}
#[test]
fn rollback_and_skip() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("s.sql");
    let a = t.path().join("01.sql");
    let b = t.path().join("02.sql");
    std::fs::write(&s, USERS).unwrap();
    std::fs::write(
        &a,
        "ALTER TABLE users DROP COLUMN email; INSERT INTO missing VALUES(1);",
    )
    .unwrap();
    std::fs::write(&b, "DROP TABLE users;").unwrap();
    let r = analyze(
        &Options {
            schema: Some(s),
            migration: vec![b, a],
            ..Default::default()
        },
        &Config::default(),
    )
    .unwrap();
    assert_eq!(r.migrations[0].status, "failed");
    assert_eq!(r.migrations[1].status, "skipped");
    assert!(!has(&r, "DP002"));
    assert!(!has(&r, "DP001"));
}
#[test]
fn authorizer_blocks_external_effects() {
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("escape.sqlite");
    for sql in [
        format!("ATTACH '{}' AS external;", target.display()),
        format!("VACUUM INTO '{}';", target.display()),
        "PRAGMA writable_schema=ON;".into(),
        "PRAGMA foreign_keys=OFF;".into(),
        "SELECT load_extension('x');".into(),
        "COMMIT;DROP TABLE users;".into(),
    ] {
        let r = run(USERS, &sql, None);
        assert!(has(&r, "MG001"), "{sql}");
        assert!(!target.exists());
    }
}
#[test]
fn quoted_identifiers() {
    let r = run(
        "CREATE TABLE \"weird\"\"table\"(\"col'quote\" TEXT);",
        "ALTER TABLE \"weird\"\"table\" DROP COLUMN \"col'quote\";",
        None,
    );
    assert!(has(&r, "MG001"));
    assert_eq!(d1_preflight::schema::quote("a\"b"), "\"a\"\"b\"");
}
#[test]
fn whitespace_and_string_normalization() {
    assert_eq!(
        d1_preflight::schema::normalized("DEFAULT ('a  b')"),
        d1_preflight::schema::normalized("DEFAULT  ( 'a  b' )")
    );
    assert_ne!(
        d1_preflight::schema::normalized("'a  b'"),
        d1_preflight::schema::normalized("'a b'")
    );
}
#[test]
fn wal_snapshot_original_unchanged() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("source.sqlite");
    let db = rusqlite::Connection::open(&p).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL;CREATE TABLE t(id INTEGER PRIMARY KEY,v TEXT);INSERT INTO t VALUES(1,'private');CREATE TABLE d1_migrations(id INTEGER PRIMARY KEY,name TEXT,applied_at TEXT);INSERT INTO d1_migrations VALUES(1,'nested/0001.sql','now');").unwrap();
    let dir = t.path().join("migrations");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("nested/0001.sql"), "CREATE TABLE t(id);").unwrap();
    std::fs::write(dir.join("0002.sql"), "ALTER TABLE t DROP COLUMN v;").unwrap();
    let before = std::fs::read(&p).unwrap();
    let r = analyze(
        &Options {
            db: Some(p.clone()),
            migrations_dir: Some(dir),
            ..Default::default()
        },
        &Config::default(),
    )
    .unwrap();
    assert_eq!(r.migrations.len(), 1);
    assert!(has(&r, "DP002"));
    assert_eq!(before, std::fs::read(&p).unwrap());
    assert_eq!(
        db.query_row("SELECT v FROM t", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "private"
    );
}
#[test]
fn schema_directory_ambiguity_and_boundary() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("s.sql");
    std::fs::write(&s, "CREATE TABLE t(id);").unwrap();
    let dir = t.path().join("m");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("01.sql"), "CREATE TABLE t(id);").unwrap();
    std::fs::write(dir.join("02.sql"), "ALTER TABLE t ADD COLUMN v;").unwrap();
    let o = Options {
        schema: Some(s),
        migrations_dir: Some(dir),
        ..Default::default()
    };
    assert!(
        analyze(&o, &Config::default())
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    let mut c = Config::default();
    c.migrations.after = Some("01.sql".into());
    assert_eq!(analyze(&o, &c).unwrap().migrations.len(), 1);
}
#[test]
fn config_rejects_unknown_and_overrides() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("config.toml");
    std::fs::write(&p, "[analysis]\nunknown=true").unwrap();
    let mut o = Options {
        schema: Some(PathBuf::from("x")),
        config: Some(p.clone()),
        ..Default::default()
    };
    assert!(Config::load(&mut o).is_err());
    std::fs::write(&p, "[analysis]\nfail_on='critical'\n[rules]\nIX001='high'").unwrap();
    o.fail_on = Some(Severity::Low);
    assert_eq!(
        Config::load(&mut o).unwrap().analysis.fail_on,
        Severity::Low
    );
}
#[test]
fn report_serialization_and_annotation_escaping() {
    let r = run(USERS, "ALTER TABLE users ADD COLUMN v;", None);
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["schema_version"], "1.0");
    assert_eq!(
        v["summary"],
        serde_json::json!({"critical":0,"high":0,"medium":0,"low":0})
    );
    assert!(r.markdown().contains("No issues detected"));
    assert_eq!(
        d1_preflight::report::escape_command("a%\n,b:c"),
        "a%25%0A%2Cb%3Ac"
    );
}

#[test]
fn temporary_database_cleanup() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sql");
    std::fs::write(&p, "CREATE TABLE t(id);").unwrap();
    let db = d1_preflight::engine::Sandbox::new(&Options {
        schema: Some(p),
        ..Default::default()
    })
    .unwrap();
    let baseline = PathBuf::from(db.baseline.path().unwrap());
    let candidate = PathBuf::from(db.candidate.path().unwrap());
    assert!(baseline.exists());
    assert!(candidate.exists());
    drop(db);
    assert!(!baseline.exists());
    assert!(!candidate.exists());
}
#[test]
fn custom_tracking_schema_and_nested_identity() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sqlite");
    let db = rusqlite::Connection::open(&p).unwrap();
    db.execute_batch("CREATE TABLE t(id);CREATE TABLE applied(NAME TEXT, arbitrary TEXT);INSERT INTO applied VALUES('nested/01.sql','x');").unwrap();
    drop(db);
    let dir = t.path().join("m");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    std::fs::write(dir.join("nested/01.sql"), "CREATE TABLE t(id);").unwrap();
    std::fs::write(dir.join("nested/02.sql"), "ALTER TABLE t ADD COLUMN v;").unwrap();
    let mut c = Config::default();
    c.migrations.tracking_table = "applied".into();
    let r = analyze(
        &Options {
            db: Some(p),
            migrations_dir: Some(dir),
            ..Default::default()
        },
        &c,
    )
    .unwrap();
    assert_eq!(r.migrations.len(), 1);
    assert_eq!(r.migrations[0].file, "nested/02.sql");
    assert!(!r.failed(Severity::High));
}
#[test]
fn unsupported_tracking_schema_fails_closed() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("s.sqlite");
    let db = rusqlite::Connection::open(&p).unwrap();
    db.execute_batch("CREATE TABLE t(id);CREATE TABLE d1_migrations(migration_id INTEGER);")
        .unwrap();
    drop(db);
    let dir = t.path().join("m");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("01.sql"), "DROP TABLE t;").unwrap();
    assert!(
        analyze(
            &Options {
                db: Some(p),
                migrations_dir: Some(dir),
                ..Default::default()
            },
            &Config::default()
        )
        .unwrap_err()
        .to_string()
        .contains("name column")
    );
}
#[test]
fn wrangler_binding_pattern_and_config_precedence() {
    let t = tempfile::tempdir().unwrap();
    let w = t.path().join("wrangler.toml");
    let p = t.path().join("preflight.toml");
    std::fs::write(&w,"[[d1_databases]]\nbinding='A'\ndatabase_name='one'\nmigrations_dir='db/migrations'\nmigrations_table='wrangler_tracking'\nmigrations_pattern='db/migrations/**/*.sql'\n[[d1_databases]]\nbinding='B'\ndatabase_name='two'\n").unwrap();
    std::fs::write(&p, "[migrations]\ntracking_table='own_tracking'\n").unwrap();
    let mut o = Options {
        schema: Some("s.sql".into()),
        wrangler_config: Some(w),
        config: Some(p),
        ..Default::default()
    };
    assert!(Config::load(&mut o).is_err());
    o.binding = Some("A".into());
    let c = Config::load(&mut o).unwrap();
    assert_eq!(c.migrations.tracking_table, "own_tracking");
    assert_eq!(o.migrations_dir, Some(t.path().join("db/migrations")));
    assert!(
        c.migrations
            .pattern
            .unwrap()
            .ends_with("db/migrations/**/*.sql")
    );
}
#[test]
fn metadata_generated_partial_expression_and_strict() {
    let mut c = rusqlite::Connection::open_in_memory().unwrap();
    d1_preflight::engine::guard(&mut c).unwrap();
    c.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY,v TEXT,g TEXT GENERATED ALWAYS AS (upper(v)) STORED) STRICT;CREATE INDEX ix ON t(lower(v)) WHERE v IS NOT NULL;CREATE TABLE wr(a TEXT PRIMARY KEY) WITHOUT ROWID;").unwrap();
    let s = d1_preflight::schema::snapshot(&c, "d1_migrations").unwrap();
    assert!(s.tables["t"].strict);
    assert_eq!(s.tables["t"].columns[2].hidden, 3);
    assert!(s.tables["t"].indexes["ix"].partial);
    assert!(s.tables["t"].indexes["ix"].terms.iter().any(|t| t.0 == -2));
    assert!(s.tables["wr"].without_rowid);
}
#[test]
fn repeated_migrations_compare_previous_rows_not_original() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("s.sql");
    let a = t.path().join("01.sql");
    let b = t.path().join("02.sql");
    std::fs::write(&s, "CREATE TABLE t(id INTEGER PRIMARY KEY);").unwrap();
    std::fs::write(&a, "INSERT INTO t VALUES(1),(2);").unwrap();
    std::fs::write(&b, "DELETE FROM t WHERE id=2;").unwrap();
    let r = analyze(
        &Options {
            schema: Some(s),
            migration: vec![a, b],
            ..Default::default()
        },
        &Config::default(),
    )
    .unwrap();
    let losses = r
        .findings
        .iter()
        .filter(|f| f.rule_id == "DP003")
        .collect::<Vec<_>>();
    assert_eq!(losses.len(), 1);
    assert!(losses[0].location.as_ref().unwrap().ends_with("02.sql"));
}
#[test]
fn invalid_baseline_query_is_not_regression() {
    let r = run(
        "CREATE TABLE t(id);",
        "ALTER TABLE t ADD COLUMN v;",
        Some("SELECT missing FROM t;"),
    );
    assert!(!has(&r, "QP002"));
    assert!(r.queries[0].baseline_error);
    assert!(
        r.limitations
            .iter()
            .any(|s| s.contains("baseline query invalid"))
    );
}
#[test]
fn stable_query_plan_does_not_report_regression() {
    let r = run(
        "CREATE TABLE t(id INTEGER PRIMARY KEY,v);CREATE INDEX ix ON t(v);",
        "ALTER TABLE t ADD COLUMN extra;",
        Some("SELECT id FROM t WHERE v=1;"),
    );
    assert!(!has(&r, "IX002"));
    assert!(!has(&r, "QP001"));
    assert!(
        r.queries[0]
            .before_steps
            .iter()
            .any(|p| p.detail.contains("SEARCH"))
    );
}
#[test]
fn expression_whitespace_does_not_change_view() {
    let r = run(
        "CREATE TABLE t(a); CREATE VIEW v AS SELECT a FROM t WHERE a='x  y';",
        "DROP VIEW v; create view v AS select a FROM t WHERE a = 'x  y';",
        None,
    );
    assert!(!has(&r, "OB001"));
}
#[test]
fn severity_override_changes_policy() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("s.sql");
    let m = t.path().join("m.sql");
    std::fs::write(&s, "CREATE TABLE t(a);CREATE INDEX ix ON t(a);").unwrap();
    std::fs::write(&m, "DROP INDEX ix;").unwrap();
    let o = Options {
        schema: Some(s),
        migration: vec![m],
        ..Default::default()
    };
    let mut c = Config::default();
    assert!(!analyze(&o, &c).unwrap().failed(Severity::High));
    c.rules.insert("IX001".into(), Severity::High);
    assert!(analyze(&o, &c).unwrap().failed(Severity::High));
}
