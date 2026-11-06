# D1 Preflight

Offline migration safety analysis for Cloudflare D1. `d1-preflight` applies pending SQL to disposable SQLite snapshots, compares schemas and row identities, validates constraints, and compares supplied workload query plans.

SQL can execute successfully while removing an application dependency, losing a uniqueness guarantee, or turning an indexed lookup into a scan. Preflight reports those consequences with evidence, severity, migration attribution where available, and review guidance.

This is a standalone Rust CLI. It does not contact Cloudflare, run Wrangler, modify the source database, deploy migrations, repair SQL, or guarantee production compatibility. SQLite evaluation approximates D1; query plans are not latency benchmarks.

## Install

Use a current stable Rust toolchain and a C compiler for bundled SQLite. SQLite is compiled with the binary; no system SQLite installation is required.

```sh
cargo build --release --locked
cargo install --path . --locked
# Or run ./target/release/d1-preflight directly.
d1-preflight --help
```

The initial build downloads crates. The compiled binary runs entirely offline, without an account, token, Wrangler, or infrastructure.

## Quick start

From this repository:

```sh
# A nullable column addition: exit 0.
cargo run -- check --schema fixtures/safe/schema.sql \
  --migration fixtures/safe/migration.sql

# A removed index causes an observed SEARCH → SCAN: exit 1.
cargo run -- check --schema fixtures/performance/schema.sql \
  --migration fixtures/performance/migration.sql \
  --queries fixtures/performance/query.sql \
  --json-out reports/preflight.json --markdown-out reports/preflight.md
```

Example terminal excerpt:

```text
D1 PREFLIGHT
SQLite 3.x | completed | 1 migrations
  Removed or changed index idx_orders_customer

HIGH IX002 — Observed query plan regression
  Evidence: before SEARCH orders USING INDEX; after SCAN orders
  Review representative data and restore or replace the supporting index.

MEDIUM IX001 — Index removed or changed
```

The actual output includes complete plan detail. The terminal uses plain text in all environments, including `NO_COLOR` and CI. JSON includes evidence and coverage; Markdown provides a concise review summary.

## Inputs and pending selection

Choose exactly one baseline source:

```sh
d1-preflight check --schema schema.sql --data fixtures/seed.sql \
  --migration migrations/0002_update_orders.sql

d1-preflight check --db snapshot.sqlite --migrations-dir migrations --queries queries
```

`--migration` is repeatable. Explicit files are ordered lexicographically by their supplied path, as are directory-relative migration identities. Use consistent zero-padded filenames. Duplicate canonical file paths are rejected.

For `--db` plus a directory, Preflight inspects the tracking table metadata and reads its `name` column (case insensitive). Exact relative path identities, including nested paths, are matched against applied entries. It never guesses from numeric IDs or a basename. A missing/unrecognized tracking table is an error: use explicit pending files or a configured `after` boundary. An empty pending set is valid when everything is applied.

A SQL baseline provides no historical boundary. Directory selection therefore requires `[migrations].after`, the exact last migration already included in that baseline. Otherwise select pending files explicitly. Never pass an entire migration history as explicit files unless the baseline predates it.

The input database is opened read-only and snapshotted through SQLite's backup API, including committed WAL contents. Reports cannot overwrite supplied source files or database sidecars. Back up production databases with your normal operational tooling before deploying.

## Findings and coverage

Implemented rules cover dropped tables/columns, row-count loss and missing primary keys, column/nullability/default/primary-key changes, uniqueness loss, foreign-key checks and relationship changes, removed indexes, workload failures and plan regressions, unbounded mutations, changed views/triggers, and execution/integrity failures. See the [rule reference](docs/rules.md).

Critical findings indicate observed destructive/incompatible outcomes or failures; high findings need review of significant compatibility risks. Medium findings identify consequential changes or potential costs. Low is available for project overrides. A rule describes its observed evidence separately from the inferred impact. A full scan or unbounded update is not automatically a mistake.

Workloads may contain multiple SELECT statements and bound parameters. Queries are planned, never executed. Named/positional parameters are bound to NULL. Non-SELECT statements are skipped explicitly. The report records invalid baseline queries and parsing gaps rather than claiming coverage.

## Configuration

`d1-preflight.toml` is loaded from the working directory when present; `--config FILE` selects another file. CLI flags override corresponding values. Paths in Preflight configuration are relative to the working directory. Unknown fields, rule IDs and severity values are rejected.

```toml
[analysis]
fail_on = "high"
check_foreign_keys = true
check_integrity = true
check_query_plans = true

[migrations]
directory = "migrations"
tracking_table = "d1_migrations"
# Required for schema + directory; optional explicit boundary for snapshots:
# after = "0001_initial.sql"

[queries]
directory = "queries"

[report]
json = "reports/preflight.json"
markdown = "reports/preflight.md"

[rules]
IX001 = "medium"
IX002 = "high"
```

Basic Wrangler TOML input is supported with `--wrangler-config wrangler.toml --binding DB`. Multiple bindings require selection. `migrations_dir`, `migrations_table`, and `migrations_pattern` are recognized. It supplies migration settings, not a database connection. See [configuration details](docs/configuration.md).

## GitHub Actions

Build the binary, select the pending migration, and retain reports even when policy fails:

```sh
./target/release/d1-preflight check \
  --schema fixtures/performance/schema.sql \
  --migration fixtures/performance/migration.sql \
  --queries fixtures/performance/query.sql \
  --json-out reports/preflight.json --markdown-out reports/preflight.md \
  --github --fail-on high
```

`--github` emits escaped annotations and appends a bounded Markdown summary when `GITHUB_STEP_SUMMARY` exists. Schema findings identify their migration file; aggregate workload findings omit migration locations when causality is uncertain. No line numbers are invented. The [example workflow](examples/preflight-workflow.yml) compiles the CLI and uploads reports. Adapt its explicit selection to your actual baseline; [repository CI](.github/workflows/ci.yml) runs checks and a passing fixture.

| Exit | Meaning                                                            |
| ---- | ------------------------------------------------------------------ |
| 0    | Analysis completed; policy passed                                  |
| 1    | Severity at or above threshold (default high), or migration failed |
| 2    | Invalid input, baseline, selection, configuration, or CLI usage    |
| 3    | Internal/output failure                                            |

Failure reports are still written when a migration fails. Execution failure always returns 1, even if severity overrides lower MG001.

## Limitations

Local SQLite does not implement the D1 service runtime. Seed rows cannot establish production safety. Statistics, parameters, engine version, and distribution affect plans. Schema comparison cannot discover all application dependencies. Explicit table/column renames are recognized for straightforward migrations; ambiguous multi-step rename chains and expression semantics need review.

Migrations are atomic per file. Explicit BEGIN/COMMIT/ROLLBACK/SAVEPOINT statements are conservatively rejected with an MG001 report. `PRAGMA defer_foreign_keys` is supported; disabling enforcement is rejected. Virtual-table creation and extensions are unsupported. Resource bounds are local tool safeguards, not D1 limits or a complete OS sandbox. Analyze untrusted SQL in a restricted container/CI environment. See [limitations and roadmap](docs/limitations.md).

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo build --release
```

If a machine has multiple Rust installations, put `~/.cargo/bin` first in PATH so `cargo`, `rustc`, `rustdoc` and Clippy use the same toolchain.

Integration and CLI tests exercise real SQLite execution, WAL backup preservation, blocked external operations, rollback, rule positives/negatives, plan changes, configuration and reports. [Fixtures](fixtures) include safe, destructive, foreign-key, uniqueness, performance, unbounded, failed and rebuild cases. [Architecture](docs/architecture.md) describes the implementation. The [orders Worker](examples/cloudflare-worker) is a separate optional demonstration without a frontend or deployment step.

Read [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md) before contributing. Preserve stable rule identifiers and report compatibility; include a reproduction and meaningful negative tests for detection changes.

## License

[MIT](LICENSE).
