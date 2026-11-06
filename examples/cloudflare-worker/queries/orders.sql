SELECT id, customer_id, created_at FROM orders WHERE customer_id = ? ORDER BY created_at DESC LIMIT 100;
