export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    if (request.method !== "GET")
      return Response.json({ error: "Method not allowed" }, { status: 405 });
    const path = new URL(request.url).pathname;
    if (path === "/health") {
      await env.DB.prepare("SELECT 1").first();
      return Response.json({ status: "ok" });
    }
    const match = /^\/customers\/(\d+)(\/orders)?$/.exec(path);
    if (!match) return Response.json({ error: "Not found" }, { status: 404 });
    const id = Number(match[1]);
    if (!Number.isSafeInteger(id))
      return Response.json({ error: "Invalid customer ID" }, { status: 400 });
    if (match[2]) {
      const { results } = await env.DB.prepare(
        "SELECT id, customer_id, created_at FROM orders WHERE customer_id = ? ORDER BY created_at DESC LIMIT 100",
      )
        .bind(id)
        .all();
      return Response.json(results);
    }
    const customer = await env.DB.prepare(
      "SELECT id, name FROM customers WHERE id = ?",
    )
      .bind(id)
      .first();
    return customer
      ? Response.json(customer)
      : Response.json({ error: "Not found" }, { status: 404 });
  },
} satisfies ExportedHandler<Env>;
