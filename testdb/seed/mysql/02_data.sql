-- sandb test seed (MySQL 8.4 + MariaDB 11): deterministic data.
-- Rows come from a cross join of digit tables, not a recursive CTE, so no
-- recursion-depth setting is needed on either engine.
SET NAMES utf8mb4;
USE shop;

CREATE TEMPORARY TABLE seq (i INT PRIMARY KEY);
INSERT INTO seq (i)
SELECT 1 + a.d + 10 * b.d + 100 * c.d + 1000 * e.d
FROM (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) a
CROSS JOIN (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) b
CROSS JOIN (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) c
CROSS JOIN (SELECT 0 d UNION ALL SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4 UNION ALL SELECT 5 UNION ALL SELECT 6 UNION ALL SELECT 7 UNION ALL SELECT 8 UNION ALL SELECT 9) e;

INSERT INTO customers (name, email, country, plan, created_at)
SELECT ELT(1 + i % 10, 'Livia Saris','Jaydon Aminoff','Maria Lubin','Ann Press','John Clarkson','Aspen Lubin','Kierra Gouse','Omar Dias','Mila Novak','Theo Grant'),
       CONCAT(LOWER(REPLACE(ELT(1 + i % 10, 'Livia Saris','Jaydon Aminoff','Maria Lubin','Ann Press','John Clarkson','Aspen Lubin','Kierra Gouse','Omar Dias','Mila Novak','Theo Grant'), ' ', '.')), '.', i, '@mail.com'),
       ELT(1 + i % 6, 'ID','US','DE','SG','JP','NL'),
       ELT(1 + i % 3, 'free','pro','team'),
       DATE_ADD('2026-01-01', INTERVAL i % 300 DAY)
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO products (sku, name, price, stock)
SELECT CONCAT('SKU-', 1000 + i),
       CONCAT(ELT(1 + i % 5, 'Desk','Lamp','Chair','Monitor','Keyboard'), ' ', i),
       5 + (i * 37 % 40000) / 100,
       CASE WHEN i % 7 = 0 THEN NULL ELSE i * 13 % 300 END
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO orders (customer_id, status, total, paid, shipped_at)
SELECT 1 + (i * 7) % 500,
       ELT(1 + i % 5, 'paid','shipped','pending','cancelled','refunded'),
       5 + (i * 7919 % 90000) / 100,
       i % 10 >= 3,
       CASE WHEN i % 5 = 1 THEN DATE_ADD('2026-10-01', INTERVAL i % 30 DAY) ELSE NULL END
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO order_items (order_id, product_id, qty, price)
SELECT 1 + (i * 3) % 500, 1 + (i * 11) % 500, 1 + i % 5, 5 + (i * 211 % 20000) / 100
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO notes (title, body, pinned, created_at)
SELECT CONCAT(ELT(1 + i % 7, 'Weekly sync','Ideas','Groceries','Reading list','Bug notes','Trip plan','Draft post'), ' #', i),
       CASE WHEN i % 9 = 0 THEN NULL ELSE ELT(1 + i % 4, 'Call Ann about Q4','Milk, eggs, coffee','Fix login timeout','Book train to Bandung') END,
       i % 10 >= 7,
       DATE_ADD('2026-10-01 09:00:00', INTERVAL i HOUR)
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO tags (name) SELECT CONCAT('tag-', i) FROM seq WHERE i <= 20 ORDER BY i;

INSERT INTO note_tags (note_id, tag_id)
SELECT 1 + (i * 17) % 500, 1 + i % 20 FROM seq WHERE i <= 120 ORDER BY i;

INSERT INTO order_status_history (order_id, seq, status, changed_at)
SELECT 1 + (i - 1) DIV 2, 1 + (i - 1) % 2,
       CASE WHEN (i - 1) % 2 = 0 THEN 'pending' ELSE 'paid' END,
       DATE_ADD('2026-10-01 00:00:00', INTERVAL i MINUTE)
FROM seq WHERE i <= 500 ORDER BY i;

INSERT INTO analytics.daily_revenue (day, orders, revenue, refunds)
SELECT DATE_ADD('2026-06-01', INTERVAL i - 1 DAY), 20 + i * 13 % 180, 5 + (i * 104729 % 2500000) / 100, (i * 811 % 90000) / 100
FROM seq WHERE i <= 120 ORDER BY i;

INSERT INTO analytics.events (user_id, name, source, created_at)
SELECT 1 + i % 500,
       ELT(1 + i % 5, 'page_view','signup','login','add_to_cart','checkout'),
       ELT(1 + i % 3, 'web','ios','android'),
       DATE_ADD('2026-10-01 00:00:00', INTERVAL i MINUTE)
FROM seq ORDER BY i;

INSERT INTO type_zoo (c_int, c_bigint, c_decimal, c_double, c_bool, c_text, c_varchar, c_uuid, c_json,
  c_date, c_datetime, c_timestamp, c_time, c_blob, c_varbinary, c_enum, c_set) VALUES
(1, 9007199254740993, 1234.5678, 3.14159, 1, 'plain text', 'varchar', 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11',
 '{"a": 1, "b": [1, 2]}', '2026-11-02', '2026-11-02 10:30:00.123456', '2026-11-02 10:30:00', '23:59:59', X'DEADBEEF', X'00FF', 'happy', 'red,blue'),
(-2147483648, -9223372036854775808, -0.0001, -1.5e300, 0, 'Unicode: café, 日本語, 🚀🎉', 'emoji 👍', '00000000-0000-0000-0000-000000000000',
 '[]', '1970-01-01', '1999-12-31 23:59:59.999999', '2000-01-01 00:00:01', '-838:59:59', X'00', X'', 'sad', ''),
(NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);

DROP TEMPORARY TABLE seq;
