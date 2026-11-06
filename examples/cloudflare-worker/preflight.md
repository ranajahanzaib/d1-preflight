# D1 Preflight

Status: **completed** · SQLite 3.54.1

2 migrations · 2 workload queries

- **high IX002** Observed query plan regression (`examples/cloudflare-worker/queries/orders.sql#1`): Review representative data and restore or replace the supporting index.
- **medium IX001** Index removed or changed (`idx_orders_customer`): Compare representative workload plans and retain an equivalent index where needed.

## Schema changes

- Added column orders.note
- Changed table definition orders
- Removed or changed index idx_orders_customer

## Coverage and limitations

- d1_foreign_key_policy: pass: enforcement enabled; deferral allowed
- d1_runtime: unverified: local SQLite approximation only
- foreign_keys: pass
- integrity: pass
- migration_execution: pass
- Local SQLite behavior is verified; D1 runtime compatibility is unverified. Passing is not deployment approval.
- Only supplied rows are tested. Application dependencies, CHECK/collation expressions and ambiguous renames require review.
- Query plans are observations, not latency measurements. Workload SQL is never executed; parameters are NULL for planning.
- orders: CHECK, collation, generated-expression and deferrability semantics require manual review of changed table SQL.
