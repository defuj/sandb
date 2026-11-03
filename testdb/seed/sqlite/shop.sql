-- sandb test seed (SQLite): builds testdb/sqlite/shop.db.
PRAGMA foreign_keys = ON;

CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT NOT NULL UNIQUE, country TEXT NOT NULL, plan TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE products (id INTEGER PRIMARY KEY, sku TEXT NOT NULL UNIQUE, name TEXT NOT NULL, price REAL NOT NULL, stock INTEGER);
CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER NOT NULL REFERENCES customers(id), status TEXT NOT NULL, total REAL NOT NULL, paid INTEGER NOT NULL, shipped_at TEXT);
CREATE TABLE order_items (id INTEGER PRIMARY KEY, order_id INTEGER NOT NULL REFERENCES orders(id), product_id INTEGER NOT NULL REFERENCES products(id), qty INTEGER NOT NULL, price REAL NOT NULL);
CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT, pinned INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
-- No primary key on purpose: edit mode must be disabled for this table.
CREATE TABLE note_tags (note_id INTEGER NOT NULL REFERENCES notes(id), tag_id INTEGER NOT NULL REFERENCES tags(id));
-- Composite primary key.
CREATE TABLE order_status_history (order_id INTEGER NOT NULL REFERENCES orders(id), seq INTEGER NOT NULL, status TEXT NOT NULL, changed_at TEXT NOT NULL, PRIMARY KEY (order_id, seq));
-- No declared primary key: only the implicit rowid identifies a row.
CREATE TABLE rowid_only (label TEXT NOT NULL, value INTEGER);
CREATE VIEW order_summary AS
  SELECT o.id, c.name AS customer, o.status, o.total, o.paid, o.shipped_at FROM orders o JOIN customers c ON c.id = o.customer_id;
CREATE TABLE daily_revenue (id INTEGER PRIMARY KEY, day TEXT NOT NULL UNIQUE, orders INTEGER NOT NULL, revenue REAL NOT NULL, refunds REAL NOT NULL);
CREATE TABLE events (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, name TEXT NOT NULL, source TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE type_zoo (id INTEGER PRIMARY KEY, c_int INTEGER, c_bigint BIGINT, c_real REAL, c_numeric NUMERIC, c_bool BOOLEAN, c_text TEXT, c_varchar VARCHAR(50), c_json TEXT, c_date DATE, c_datetime DATETIME, c_blob BLOB);

CREATE TEMP TABLE seq (i INTEGER PRIMARY KEY);
WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < 10000) INSERT INTO seq SELECT i FROM s;
CREATE TEMP TABLE nm (k INTEGER PRIMARY KEY, v TEXT);
INSERT INTO nm VALUES (0,'Livia Saris'),(1,'Jaydon Aminoff'),(2,'Maria Lubin'),(3,'Ann Press'),(4,'John Clarkson'),(5,'Aspen Lubin'),(6,'Kierra Gouse'),(7,'Omar Dias'),(8,'Mila Novak'),(9,'Theo Grant');

INSERT INTO customers (id, name, email, country, plan, created_at)
SELECT i, nm.v, lower(replace(nm.v, ' ', '.')) || '.' || i || '@mail.com',
  CASE i % 6 WHEN 0 THEN 'ID' WHEN 1 THEN 'US' WHEN 2 THEN 'DE' WHEN 3 THEN 'SG' WHEN 4 THEN 'JP' ELSE 'NL' END,
  CASE i % 3 WHEN 0 THEN 'free' WHEN 1 THEN 'pro' ELSE 'team' END,
  date('2026-01-01', '+' || (i % 300) || ' days')
FROM seq JOIN nm ON nm.k = i % 10 WHERE i <= 500;

INSERT INTO products (id, sku, name, price, stock)
SELECT i, 'SKU-' || (1000 + i),
  CASE i % 5 WHEN 0 THEN 'Desk' WHEN 1 THEN 'Lamp' WHEN 2 THEN 'Chair' WHEN 3 THEN 'Monitor' ELSE 'Keyboard' END || ' ' || i,
  5 + (i * 37 % 40000) / 100.0, CASE WHEN i % 7 = 0 THEN NULL ELSE i * 13 % 300 END
FROM seq WHERE i <= 500;

INSERT INTO orders (id, customer_id, status, total, paid, shipped_at)
SELECT i, 1 + (i * 7) % 500,
  CASE i % 5 WHEN 0 THEN 'paid' WHEN 1 THEN 'shipped' WHEN 2 THEN 'pending' WHEN 3 THEN 'cancelled' ELSE 'refunded' END,
  5 + (i * 7919 % 90000) / 100.0, i % 10 >= 3,
  CASE WHEN i % 5 = 1 THEN date('2026-10-01', '+' || (i % 30) || ' days') END
FROM seq WHERE i <= 500;

INSERT INTO order_items (id, order_id, product_id, qty, price)
SELECT i, 1 + (i * 3) % 500, 1 + (i * 11) % 500, 1 + i % 5, 5 + (i * 211 % 20000) / 100.0 FROM seq WHERE i <= 500;

INSERT INTO notes (id, title, body, pinned, created_at)
SELECT i,
  CASE i % 7 WHEN 0 THEN 'Weekly sync' WHEN 1 THEN 'Ideas' WHEN 2 THEN 'Groceries' WHEN 3 THEN 'Reading list' WHEN 4 THEN 'Bug notes' WHEN 5 THEN 'Trip plan' ELSE 'Draft post' END || ' #' || i,
  CASE WHEN i % 9 = 0 THEN NULL ELSE CASE i % 4 WHEN 0 THEN 'Call Ann about Q4' WHEN 1 THEN 'Milk, eggs, coffee' WHEN 2 THEN 'Fix login timeout' ELSE 'Book train to Bandung' END END,
  i % 10 >= 7, datetime('2026-10-01 09:00:00', '+' || i || ' hours')
FROM seq WHERE i <= 500;

INSERT INTO tags (id, name) SELECT i, 'tag-' || i FROM seq WHERE i <= 20;
INSERT INTO note_tags (note_id, tag_id) SELECT 1 + (i * 17) % 500, 1 + i % 20 FROM seq WHERE i <= 120;
INSERT INTO order_status_history (order_id, seq, status, changed_at)
SELECT 1 + (i - 1) / 2, 1 + (i - 1) % 2, CASE WHEN (i - 1) % 2 = 0 THEN 'pending' ELSE 'paid' END,
  datetime('2026-10-01 00:00:00', '+' || i || ' minutes')
FROM seq WHERE i <= 500;
INSERT INTO rowid_only (label, value) SELECT 'row ' || i, CASE WHEN i % 4 = 0 THEN NULL ELSE i * 10 END FROM seq WHERE i <= 50;

INSERT INTO daily_revenue (id, day, orders, revenue, refunds)
SELECT i, date('2026-06-01', '+' || (i - 1) || ' days'), 20 + i * 13 % 180, 5 + (i * 104729 % 2500000) / 100.0, (i * 811 % 90000) / 100.0
FROM seq WHERE i <= 120;

INSERT INTO events (id, user_id, name, source, created_at)
SELECT i, 1 + i % 500,
  CASE i % 5 WHEN 0 THEN 'page_view' WHEN 1 THEN 'signup' WHEN 2 THEN 'login' WHEN 3 THEN 'add_to_cart' ELSE 'checkout' END,
  CASE i % 3 WHEN 0 THEN 'web' WHEN 1 THEN 'ios' ELSE 'android' END,
  datetime('2026-10-01 00:00:00', '+' || i || ' minutes')
FROM seq;

INSERT INTO type_zoo (c_int, c_bigint, c_real, c_numeric, c_bool, c_text, c_varchar, c_json, c_date, c_datetime, c_blob) VALUES
(1, 9007199254740993, 3.14159, 1234.5678, 1, 'plain text', 'varchar', '{"a": 1, "b": [1, 2]}', '2026-11-02', '2026-11-02 10:30:00', X'DEADBEEF'),
(-2147483648, -9223372036854775808, -1.5e300, -0.0001, 0, 'Unicode: café, 日本語, 🚀🎉', 'emoji 👍', '[]', '1970-01-01', '1999-12-31 23:59:59', X'00'),
(NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);

DROP TABLE seq;
DROP TABLE nm;
