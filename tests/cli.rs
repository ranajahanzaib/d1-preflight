use std::process::{Command, Output};
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_d1-preflight"))
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn help_and_usage() {
    assert!(cli(&["--help"]).status.success());
    assert_eq!(cli(&["check"]).status.code(), Some(2));
    assert_eq!(
        cli(&["check", "--schema", "a", "--db", "b"]).status.code(),
        Some(2)
    );
    assert_eq!(
        cli(&["check", "--schema", "missing", "--migration", "missing"])
            .status
            .code(),
        Some(2)
    );
}
#[test]
fn policy_and_reports() {
    let t = tempfile::tempdir().unwrap();
    let json = t.path().join("reports/out.json");
    let md = t.path().join("out.md");
    let output = cli(&[
        "check",
        "--schema",
        "fixtures/performance/schema.sql",
        "--migration",
        "fixtures/performance/migration.sql",
        "--queries",
        "fixtures/performance/query.sql",
        "--json-out",
        json.to_str().unwrap(),
        "--markdown-out",
        md.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1), "{:?}", output);
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
    assert_eq!(v["schema_version"], "1.0");
    assert!(std::fs::read_to_string(md).unwrap().contains("IX002"));
    assert!(
        cli(&[
            "check",
            "--schema",
            "fixtures/safe/schema.sql",
            "--migration",
            "fixtures/safe/migration.sql"
        ])
        .status
        .success()
    );
    assert!(
        cli(&[
            "check",
            "--schema",
            "fixtures/performance/schema.sql",
            "--migration",
            "fixtures/performance/migration.sql",
            "--fail-on",
            "critical"
        ])
        .status
        .success()
    );
}
#[test]
fn failed_migration_still_reports() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("failed.json");
    let o = cli(&[
        "check",
        "--schema",
        "fixtures/failed/schema.sql",
        "--migration",
        "fixtures/failed/migration.sql",
        "--json-out",
        p.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert_eq!(v["status"], "migration_failed");
    assert_eq!(v["findings"][0]["rule_id"], "MG001");
}
#[test]
fn output_error_is_internal() {
    let t = tempfile::tempdir().unwrap();
    let output = cli(&[
        "check",
        "--schema",
        "fixtures/safe/schema.sql",
        "--migration",
        "fixtures/safe/migration.sql",
        "--json-out",
        t.path().to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(3));
}
#[test]
fn outputs_cannot_overwrite_input() {
    let t = tempfile::tempdir().unwrap();
    let s = t.path().join("schema.sql");
    std::fs::write(&s, "CREATE TABLE users(id);").unwrap();
    let bytes = std::fs::read(&s).unwrap();
    let o = cli(&[
        "check",
        "--schema",
        s.to_str().unwrap(),
        "--migration",
        "fixtures/safe/migration.sql",
        "--json-out",
        s.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(bytes, std::fs::read(s).unwrap());
}
#[test]
fn github_escapes_and_summary() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("summary.md");
    std::fs::write(&p, "").unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_d1-preflight"))
        .args([
            "check",
            "--schema",
            "fixtures/destructive/schema.sql",
            "--migration",
            "fixtures/destructive/migration.sql",
            "--github",
        ])
        .env("GITHUB_STEP_SUMMARY", &p)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(
        String::from_utf8(o.stdout)
            .unwrap()
            .contains("::error file=")
    );
    assert!(std::fs::read_to_string(p).unwrap().contains("DP002"));
}
#[test]
fn stable_safe_report_golden() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("r.json");
    assert!(
        cli(&[
            "check",
            "--schema",
            "fixtures/safe/schema.sql",
            "--migration",
            "fixtures/safe/migration.sql",
            "--json-out",
            p.to_str().unwrap()
        ])
        .status
        .success()
    );
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    v.as_object_mut().unwrap().remove("sqlite_version");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("golden/safe.json")).unwrap();
    assert_eq!(v, expected);
}

#[test]
#[cfg(unix)]
fn report_symlink_and_hardlink_aliases_are_rejected() {
    use std::os::unix::fs::symlink;
    let t = tempfile::tempdir().unwrap();
    let input = t.path().join("s.sql");
    std::fs::write(&input, "CREATE TABLE t(a);").unwrap();
    let symlink_path = t.path().join("symlink.json");
    let hardlink_path = t.path().join("hardlink.json");
    symlink(&input, &symlink_path).unwrap();
    std::fs::hard_link(&input, &hardlink_path).unwrap();
    for output in [symlink_path, hardlink_path] {
        let o = cli(&[
            "check",
            "--schema",
            input.to_str().unwrap(),
            "--migration",
            "fixtures/safe/migration.sql",
            "--json-out",
            output.to_str().unwrap(),
        ]);
        assert_eq!(o.status.code(), Some(2));
    }
    assert_eq!(
        std::fs::read_to_string(input).unwrap(),
        "CREATE TABLE t(a);"
    );
}
#[test]
fn migration_error_has_sqlite_diagnostic_codes_without_literals() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("r.json");
    let o = cli(&[
        "check",
        "--schema",
        "fixtures/failed/schema.sql",
        "--migration",
        "fixtures/failed/migration.sql",
        "--json-out",
        p.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
    assert_eq!(
        v["findings"][0]["evidence"]["sqlite_error"]["extended_code"],
        1
    );
}

#[test]
fn oversized_source_is_invalid_input_not_internal_failure() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("large.sql");
    let f = std::fs::File::create(&p).unwrap();
    f.set_len(4 * 1024 * 1024 + 1).unwrap();
    assert_eq!(
        cli(&[
            "check",
            "--schema",
            p.to_str().unwrap(),
            "--migration",
            "fixtures/safe/migration.sql"
        ])
        .status
        .code(),
        Some(2)
    );
}
