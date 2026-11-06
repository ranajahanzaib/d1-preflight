# Architecture

One crate exposes a reusable analysis library and a thin Clap binary. `config` parses and validates inputs; `engine` handles selection, backups, guards, and bounded SQL loading; `schema` extracts metadata; `analysis` emits typed findings; `compatibility` verifies the D1 enforcement policy while explicitly marking runtime behavior unverified; `report` renders them without duplicating detection logic.

## Database lifecycle

A temporary directory owns baseline and candidate SQLite files. SQL baselines are executed under a SQLite authorizer, followed by optional data. Existing databases are opened read-only and copied through incremental `Backup::step`, with a 30-second deadline and a 3-second busy timeout. This observes committed WAL data through SQLite instead of copying a live main file.

The candidate is backed up from the baseline. A temporary previous-state snapshot is made before every pending migration to compare row identity survival per file. Connections are dropped before their temporary owners. Nothing writes SQL to the supplied database. Opening a live WAL source can require access to SQLite-managed shared-memory files; the CLI makes no source SQL writes.

Each migration runs in an implicit transaction with foreign keys enabled. Its authorizer denies transaction control, ATTACH/DETACH (including VACUUM INTO), virtual-table operations, extension/file functions and unsafe PRAGMA assignments. Explicit transactions are rejected rather than rewritten. A failed execution/commit rolls back; dependent files are recorded as skipped. Failed candidates are not compared as successful migrations. Constraint evidence observed before a failed deferred commit is retained, while final checks describe the rolled-back candidate.

The tool uses SQL-length/value limits, a page-count bound, progress-handler deadlines and temporary storage in memory. These are approximate resource guards, not a security boundary; use OS isolation for adversarial SQL. The SQL parser handles static structure only; SQLite remains the execution authority. Unsupported parser syntax produces coverage limitations.

## Metadata and comparison

`sqlite_schema`, table-valued `table_xinfo`, `table_list`, `index_list`, `index_xinfo`, and `foreign_key_list` expose structured metadata with bound identifiers; quoted identifiers are used for row-count queries. Models retain primary-key order, generated/hidden metadata, index terms/collations/partial SQL, relationship actions, views/triggers, STRICT and WITHOUT ROWID flags. SQLite, `_cf_` and selected tracking objects are excluded.

Ordered maps make comparisons deterministic. Final structures are compared per successful migration; intermediate DROP statements in a preserving rebuild do not automatically become dropped-table findings. SQL token normalization removes whitespace/comments while preserving quoted values and expression punctuation. It is not semantic equivalence analysis. Unmodeled changed table expressions are called out for manual review.

Dropped columns remain compatibility findings even with zero rows. Row-count decreases prove a minimum row loss. With an unchanged usable primary-key projection and bounded data, missing identities are counted privately and reported without key values. Missing identities can mean deletion or intentional re-keying. Without usable identities, equal counts prove nothing; coverage is explicitly limited. Value-by-value row preservation is not asserted.

## Workloads and reporting

SQLparser establishes SELECT statement boundaries; each statement is prepared using EXPLAIN QUERY PLAN against baseline and final candidate; structured node IDs, parent IDs and details are retained. Parameters are NULL; workload statements themselves are never executed. Indexed SEARCH becoming a full SCAN for the same plan table/alias, new sort B-trees, and new automatic indexes produce plan-risk findings. A scan without observed regression is only a potential cost. Query errors expose no raw SQL literals.

Typed findings carry stable IDs, observed evidence, confidence, recommendations and per-file source attribution. Workload changes across multiple migrations deliberately omit a guessed migration. Findings sort by descending severity, rule, object and file; schema changes and coverage limitations sort/deduplicate. There are no timestamps or disposable database paths. JSON schema version 1.0 identifies this MVP serialization; future breaking changes must increment it. Raw SQLite execution diagnostics are redacted because they can contain private values; failure classes and status remain available.
