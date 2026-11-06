# Configuration

See the complete example in the README. TOML fields are strictly checked. Every listed rule accepts `low`, `medium`, `high`, or `critical`; no suppressions or disabled severity exist in this MVP.

- `analysis.fail_on`: default high; CLI `--fail-on` overrides it. Findings equal to or above the threshold fail policy.
- `analysis.check_foreign_keys`, `check_integrity`, `check_query_plans`: default true. Disabled checks are explicitly marked skipped; SQL engine FK enforcement is never disabled.
- `migrations.directory`: optional discovery root, overridden by `--migrations-dir`.
- `migrations.tracking_table`: default `d1_migrations`. Must exist and have an actual `name` column for automatic snapshot selection. Unsupported tracking schemas require explicit files or an `after` boundary.
- `migrations.after`: exact relative path of the last already-applied discovered migration. Only lexicographically later files are pending. It must exist in discovery. This overrides tracking-based selection intentionally; verify the boundary against your baseline.
- `migrations.pattern`: optional glob; matches must remain under `directory` and are sorted by relative identity. Without a pattern, SQL files are recursively discovered.
- `queries.directory`: file or directory, overridden by `--queries`.
- `report.json`, `report.markdown`: destinations, overridden by CLI output flags. Parent directories are created.
- `rules`: severity overrides keyed by stable rule ID. Unknown IDs are errors.

Configuration paths are relative to the working directory, not the config file. Explicit CLI migrations add to configured directory selection; do not combine them with overlapping discovery. Duplicate canonical paths fail.

## Wrangler TOML

Pass `--wrangler-config FILE` and, for multiple D1 entries, `--binding NAME`. The CLI reads `d1_databases`, `binding`, `migrations_dir`, `migrations_table`, and `migrations_pattern`; `database_name` remains an informational Wrangler field, never a connection target. Directory/pattern paths from Wrangler are relative to that file. The pattern must include the migration directory prefix, matching Wrangler conventions. Preflight configuration directory/pattern values take precedence, and CLI directory flags take final precedence.

Wrangler JSON/JSONC, environment inheritance and remote/local state discovery are unsupported. Export a schema or explicitly supply a local SQLite snapshot yourself. A placeholder or real `database_id` never grants remote access.

[Cloudflare's migration documentation](https://developers.cloudflare.com/d1/reference/migrations/) defines relative path identities and `migrations_pattern`. [D1 foreign-key documentation](https://developers.cloudflare.com/d1/sql-api/foreign-keys/) describes always-on enforcement and `defer_foreign_keys`; the local engine follows that enforcement policy without claiming exact runtime parity.
