-- sandb test seed (MySQL 8.4 + MariaDB 11): schema.
SET NAMES utf8mb4;
CREATE DATABASE shop CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE analytics CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

USE shop;

CREATE TABLE customers (
  id         INT AUTO_INCREMENT PRIMARY KEY,
  name       VARCHAR(100) NOT NULL,
  email      VARCHAR(190) NOT NULL UNIQUE,
  country    CHAR(2) NOT NULL,
  plan       VARCHAR(20) NOT NULL,
  created_at DATE NOT NULL
);

CREATE TABLE products (
  id    INT AUTO_INCREMENT PRIMARY KEY,
  sku   VARCHAR(30) NOT NULL UNIQUE,
  name  VARCHAR(100) NOT NULL,
  price DECIMAL(10,2) NOT NULL,
  stock INT NULL
);

CREATE TABLE orders (
  id          INT AUTO_INCREMENT PRIMARY KEY,
  customer_id INT NOT NULL,
  status      VARCHAR(20) NOT NULL,
  total       DECIMAL(10,2) NOT NULL,
  paid        TINYINT(1) NOT NULL,
  shipped_at  DATE NULL,
  FOREIGN KEY (customer_id) REFERENCES customers(id)
);

CREATE TABLE order_items (
  id         INT AUTO_INCREMENT PRIMARY KEY,
  order_id   INT NOT NULL,
  product_id INT NOT NULL,
  qty        INT NOT NULL,
  price      DECIMAL(10,2) NOT NULL,
  FOREIGN KEY (order_id) REFERENCES orders(id),
  FOREIGN KEY (product_id) REFERENCES products(id)
);

CREATE TABLE notes (
  id         INT AUTO_INCREMENT PRIMARY KEY,
  title      VARCHAR(200) NOT NULL,
  body       TEXT NULL,
  pinned     TINYINT(1) NOT NULL DEFAULT 0,
  created_at DATETIME NOT NULL
);

CREATE TABLE tags (
  id   INT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(50) NOT NULL UNIQUE
);

-- No primary key on purpose: edit mode must be disabled for this table.
CREATE TABLE note_tags (
  note_id INT NOT NULL,
  tag_id  INT NOT NULL,
  FOREIGN KEY (note_id) REFERENCES notes(id),
  FOREIGN KEY (tag_id) REFERENCES tags(id)
);

-- Composite primary key.
CREATE TABLE order_status_history (
  order_id   INT NOT NULL,
  seq        INT NOT NULL,
  status     VARCHAR(20) NOT NULL,
  changed_at DATETIME NOT NULL,
  PRIMARY KEY (order_id, seq),
  FOREIGN KEY (order_id) REFERENCES orders(id)
);

CREATE VIEW order_summary AS
  SELECT o.id, c.name AS customer, o.status, o.total, o.paid, o.shipped_at
  FROM orders o JOIN customers c ON c.id = o.customer_id;

CREATE TABLE type_zoo (
  id          INT AUTO_INCREMENT PRIMARY KEY,
  c_int       INT NULL,
  c_bigint    BIGINT NULL,
  c_decimal   DECIMAL(12,4) NULL,
  c_double    DOUBLE NULL,
  c_bool      TINYINT(1) NULL,
  c_text      TEXT NULL,
  c_varchar   VARCHAR(50) NULL,
  c_uuid      CHAR(36) NULL,
  c_json      JSON NULL,
  c_date      DATE NULL,
  c_datetime  DATETIME(6) NULL,
  c_timestamp TIMESTAMP NULL DEFAULT NULL,
  c_time      TIME NULL,
  c_blob      BLOB NULL,
  c_varbinary VARBINARY(16) NULL,
  c_enum      ENUM('sad','ok','happy') NULL,
  c_set       SET('red','green','blue') NULL
);

USE analytics;

CREATE TABLE daily_revenue (
  id      INT AUTO_INCREMENT PRIMARY KEY,
  day     DATE NOT NULL UNIQUE,
  orders  INT NOT NULL,
  revenue DECIMAL(12,2) NOT NULL,
  refunds DECIMAL(12,2) NOT NULL
);

CREATE TABLE events (
  id         INT AUTO_INCREMENT PRIMARY KEY,
  user_id    INT NOT NULL,
  name       VARCHAR(30) NOT NULL,
  source     VARCHAR(20) NOT NULL,
  created_at DATETIME NOT NULL
);
