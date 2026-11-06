# Orders Worker example

The baseline represents the schema before migration 0002. Migration 0002 adds a nullable note; 0003 deliberately removes an index used by the orders workload. These are pending examples, not a complete initialization history.

From the repository root, with no Wrangler or account required:

```sh
cargo run -- check \
  --schema examples/cloudflare-worker/schema.sql \
  --data examples/cloudflare-worker/seed.sql \
  --migration examples/cloudflare-worker/migrations/0002_add_note.sql \
  --migration examples/cloudflare-worker/migrations/0003_drop_index.sql \
  --queries examples/cloudflare-worker/queries \
  --json-out reports/orders.json --markdown-out reports/orders.md
```

Exit 1 is expected. Committed `preflight.json` and `preflight.md` show the observed local plans. Parameters are bound to NULL during planning; production distributions can change those plans.

Optional local Worker testing (Node.js and pnpm required), from this directory:

```sh
pnpm install --frozen-lockfile
pnpm run types
pnpm run typecheck
npx wrangler d1 execute preflight-orders --local --file schema.sql
npx wrangler d1 execute preflight-orders --local --file seed.sql
npx wrangler d1 migrations apply preflight-orders --local
pnpm run dev
# In another terminal:
curl http://localhost:8787/health
curl http://localhost:8787/customers/1
curl http://localhost:8787/customers/1/orders
```

The compatibility date is pinned to the latest date supported by the locked local runtime. The placeholder database ID is for local development. No deployment is performed. If restarting from an existing local database, do not replay schema.sql; inspect migration state first. See [Cloudflare D1 migration conventions](https://developers.cloudflare.com/d1/reference/migrations/) and [foreign key semantics](https://developers.cloudflare.com/d1/sql-api/foreign-keys/).
