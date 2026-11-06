# Limitations and roadmap

- The bundled local SQLite version is included in each report. D1 execution policy, functions, service limits, engine versions and implicit transactions can differ. No D1 size/parameter quota is asserted from unverified assumptions.
- A passing report is not deployment approval. Only supplied data and workload SQL are covered; synthetic rows cannot demonstrate production safety or identify all application dependencies.
- Workloads are SELECT only. Non-SELECT or unparseable input is recorded as skipped. Query plans depend on data, statistics, parameter values and engine version; parameters here are NULL. A scan may be optimal. No timings or slowdown factors are inferred.
- Per-file transactions are supported. Explicit transaction control is conservatively rejected, with structured failure output. This avoids rewriting scripts and incorrectly handling deferred constraints. SQL parser gaps are coverage limitations; the SQLite authorizer independently blocks transaction escape.
- Disabling foreign keys is rejected. Deferral is allowed; integrity checks still run. CHECK expressions, conflict strategies, collations and FK deferrability are not fully semantically modeled. Changed table SQL is retained as a manual-review limitation.
- Simple explicit renames are recognized. Rename chains, reusing names and obscure quoting can remain ambiguous. Similar names are never enough to confirm a rename. Removing and recreating an empty table under the same name is judged from final structure, not automatically called permanent loss.
- Row counts and primary-key survival do not establish value preservation. Row identity matching is bounded to 100,000 rows and usable non-null keys. Missing keys may represent intentional re-keying. No stable identity means preservation is unverified. Data moves between unrelated tables cannot be proven equivalent.
- SQL file size is bounded to 4 MiB and individual SQLite values to 16 MiB; candidate files are limited to 262,144 SQLite pages. SQLite execution has a 30-second progress deadline per guard scope; snapshots have a 30-second deadline. Sorting, parsing, metadata allocation and third-party library operations are not a complete memory/time sandbox. Use a process/container memory limit for large or untrusted inputs.
- Authorization blocks extension loading, virtual-table creation, external database attachment, known file functions, unsafe PRAGMA assignments and transaction escape. It does not make arbitrary SQL safe against SQLite vulnerabilities, OS attacks, or resource exhaustion. Use restricted CI/container execution for untrusted inputs. No shell commands are derived from SQL.
- Snapshotting opens the source read-only and uses backup APIs, including committed WAL state; concurrent writes after the snapshot are outside coverage. SQLite may need source WAL/SHM access. Production backup and deployment concurrency controls remain your responsibility.
- Reports never serialize row result values or primary-key contents (FK diagnostics may include SQLite row identifiers). Raw errors are redacted because SQLite can include private values. Schema definitions, object names, query plan details and supplied file paths remain visible; schema defaults themselves may contain sensitive constants.
- GitHub annotations omit guessed line numbers. Markdown summaries are concise; JSON is the authoritative detailed report. Aggregate workload changes omit migration attribution when not proven.
- Wrangler TOML is basic only: no JSONC, environments or automatic local-state discovery. Tracking uses a metadata-verified `name` column; other tracking formats require explicit selection.

## Possible follow-ups

**v0.2:** More precise locations, broader Wrangler support, improved parsing and rename/rebuild matching, other migration frameworks, justified suppressions.

**v0.3:** SARIF, opt-in pull-request comments, workload collection helpers, broader D1 compatibility checks and Wrangler parity tests.

**v1.0:** Stable configuration/report contracts, wider SQLite feature coverage, performance calibration and cross-version compatibility verification. These are future work, not current guarantees.
