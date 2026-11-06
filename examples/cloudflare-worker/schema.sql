CREATE TABLE customers(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE orders(id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), created_at TEXT NOT NULL);
CREATE INDEX idx_orders_customer ON orders(customer_id, created_at DESC);
