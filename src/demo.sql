PRAGMA foreign_keys = ON;
CREATE TABLE customers (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    plan TEXT NOT NULL DEFAULT 'Starter',
    status TEXT NOT NULL DEFAULT 'Active',
    country TEXT,
    lifetime_value REAL NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2400)
INSERT INTO customers
SELECT x,
    CASE x % 8 WHEN 1 THEN 'Avery Chen' WHEN 2 THEN 'Noah Williams' WHEN 3 THEN 'Sofia Martinez' WHEN 4 THEN 'Liam Anderson' WHEN 5 THEN 'Isla Thompson' WHEN 6 THEN 'Oliver Park' WHEN 7 THEN 'Amelia Wilson' ELSE 'James Taylor' END,
    'customer.' || x || '@example.com',
    CASE x % 3 WHEN 0 THEN 'Enterprise' WHEN 1 THEN 'Pro' ELSE 'Starter' END,
    CASE WHEN x % 9 = 0 THEN 'Inactive' ELSE 'Active' END,
    CASE x % 5 WHEN 0 THEN 'Netherlands' WHEN 1 THEN 'United States' WHEN 2 THEN 'United Kingdom' WHEN 3 THEN 'Germany' ELSE 'Canada' END,
    round((x * 139.73) % 12000 + 49.0, 2),
    datetime('2026-01-01', '+' || x || ' hours')
FROM n;
CREATE TABLE products (id INTEGER PRIMARY KEY, name TEXT NOT NULL, price REAL NOT NULL, inventory INTEGER NOT NULL DEFAULT 0);
INSERT INTO products VALUES (1, 'Studio annual subscription', 249, 999), (2, 'Team annual subscription', 799, 999), (3, 'Developer toolkit', 79, 245), (4, 'Workspace add-on', 29, 999);
CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), product_id INTEGER NOT NULL REFERENCES products(id), total REAL NOT NULL, status TEXT NOT NULL, created_at TEXT NOT NULL);
INSERT INTO orders SELECT id, id, id % 4 + 1, lifetime_value, CASE WHEN id % 7 = 0 THEN 'Pending' ELSE 'Paid' END, created_at FROM customers WHERE id <= 1800;
CREATE INDEX orders_customer_id ON orders(customer_id);
CREATE TABLE events (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customers(id), event TEXT NOT NULL, metadata TEXT, created_at TEXT NOT NULL);
INSERT INTO events SELECT id, id, 'account.created', '{"source":"website"}', created_at FROM customers;
CREATE VIEW revenue_by_plan AS SELECT plan, count(*) AS customers, round(sum(lifetime_value), 2) AS revenue FROM customers GROUP BY plan;
