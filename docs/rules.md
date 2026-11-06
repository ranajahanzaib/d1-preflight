# Rule reference

Rules use actual SQLite metadata, execution or query-plan evidence. Severity expresses review priority, not proof of a production incident. Static unbounded mutation findings express possible risk; plan changes are observations rather than measured slowdowns.

| ID    | Default  | Trigger and evidence                                                                    | Review action                                      |
| ----- | -------- | --------------------------------------------------------------------------------------- | -------------------------------------------------- |
| DP001 | critical | Table absent after successful migration; supplied baseline row count                    | Migrate readers; preserve required records         |
| DP002 | critical | Column absent in final table, regardless of row count; old metadata                     | Coordinate application changes and retention       |
| DP003 | critical | Lower row count, or disappearing bounded stable primary keys; counts only               | Fix copy predicates or verify deliberate re-keying |
| SC001 | high     | Type, hidden/generated metadata, nullability, STRICT or WITHOUT ROWID changes           | Validate writers and storage assumptions           |
| SC002 | high     | Existing/new column becomes required; before/after metadata                             | Backfill data and update writers                   |
| SC003 | high     | Ordered primary-key projection changes                                                  | Preserve identity and relationships                |
| SC004 | medium   | Default SQL tokens differ                                                               | Verify omitted-value insert behavior               |
| SC005 | high     | Unique index/constraint lacks an equivalent final unique term set and partial predicate | Preserve uniqueness guarantees                     |
| FK001 | critical | Actual foreign_key_check violations or invalid relationship metadata                    | Repair orphaned rows and relationship schema       |
| FK002 | high     | Existing foreign-key definitions removed/altered, including actions                     | Review enforcement and cascading effects           |
| IX001 | medium   | Index removed or uniqueness, terms, collation, order, expression or partial SQL changes | Compare workload requirements                      |
| IX002 | high     | Same table/alias SEARCH becomes full SCAN, new temporary B-tree or automatic index      | Review representative plans and supporting indexes |
| QP001 | medium   | Candidate full scan without a confirmed regression                                      | Assess table size and query selectivity            |
| QP002 | critical | A workload SELECT prepares before but fails after                                       | Restore dependencies or update SQL                 |
| SQ001 | high     | Parsed top-level UPDATE/DELETE has no WHERE; available pre-file count                   | Review scope; not automatically an error           |
| OB001 | medium   | View or trigger removed/changed in final schema; token-preserving SQL comparison        | Audit consumers and side effects                   |
| MG001 | critical | Migration/commit rejected, unsupported explicit transaction, or failed integrity check  | Correct failure before deploying                   |

Limitations: changed CHECK, collation and generated-expression semantics are flagged as unverified table-definition changes rather than falsely certified. Unique replacement matching is conservative, especially partial/expression indexes. Equivalent index renaming can still generate IX001 (review) without SC005. Relationship reordering may generate FK002 for review. Explicit straightforward renames suppress incorrect removal findings; unsupported rename chains remain uncertain.

Immediate FK enforcement may reject SQL before foreign_key_check can observe orphan rows; MG001 then records the actual execution failure. Deferred violations are observed before commit and produce FK001 as well. Final checks after rollback describe the surviving candidate, not the rejected intermediate state.
