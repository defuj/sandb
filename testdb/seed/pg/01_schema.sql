-- sandb test seed (PostgreSQL): schema. Runs as postgres in database "shop".
CREATE SCHEMA analytics;

CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

CREATE TABLE customers (
  id         serial PRIMARY KEY,
  name       text NOT NULL,
  email      text NOT NULL UNIQUE,
  country    char(2) NOT NULL,
  plan       text NOT NULL,
  created_at date NOT NULL
);

CREATE TABLE products (
  id    serial PRIMARY KEY,
  sku   text NOT NULL UNIQUE,
  name  text NOT NULL,
  price numeric(10,2) NOT NULL,
  stock integer
);

CREATE TABLE orders (
  id          serial PRIMARY KEY,
  customer_id integer NOT NULL REFERENCES customers(id),
  status      text NOT NULL,
  total       numeric(10,2) NOT NULL,
  paid        boolean NOT NULL,
  shipped_at  date
);

CREATE TABLE order_items (
  id         serial PRIMARY KEY,
  order_id   integer NOT NULL REFERENCES orders(id),
  product_id integer NOT NULL REFERENCES products(id),
  qty        integer NOT NULL,
  price      numeric(10,2) NOT NULL
);

CREATE TABLE notes (
  id         serial PRIMARY KEY,
  title      text NOT NULL,
  body       text,
  pinned     boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL
);

CREATE TABLE tags (
  id   serial PRIMARY KEY,
  name text NOT NULL UNIQUE
);

-- No primary key on purpose: edit mode must be disabled for this table.
CREATE TABLE note_tags (
  note_id integer NOT NULL REFERENCES notes(id),
  tag_id  integer NOT NULL REFERENCES tags(id)
);

-- Composite primary key.
CREATE TABLE order_status_history (
  order_id   integer NOT NULL REFERENCES orders(id),
  seq        integer NOT NULL,
  status     text NOT NULL,
  changed_at timestamptz NOT NULL,
  PRIMARY KEY (order_id, seq)
);

CREATE VIEW order_summary AS
  SELECT o.id, c.name AS customer, o.status, o.total, o.paid, o.shipped_at
  FROM orders o JOIN customers c ON c.id = o.customer_id;

CREATE TABLE analytics.daily_revenue (
  id      serial PRIMARY KEY,
  day     date NOT NULL UNIQUE,
  orders  integer NOT NULL,
  revenue numeric(12,2) NOT NULL,
  refunds numeric(12,2) NOT NULL
);

CREATE TABLE analytics.events (
  id         serial PRIMARY KEY,
  user_id    integer NOT NULL,
  name       text NOT NULL,
  source     text NOT NULL,
  created_at timestamptz NOT NULL
);

CREATE TABLE type_zoo (
  id           serial PRIMARY KEY,
  c_int        integer,
  c_bigint     bigint,
  c_numeric    numeric(12,4),
  c_float      double precision,
  c_bool       boolean,
  c_text       text,
  c_varchar    varchar(50),
  c_uuid       uuid,
  c_json       json,
  c_jsonb      jsonb,
  c_date       date,
  c_timestamp  timestamp,
  c_timestamptz timestamptz,
  c_time       time,
  c_interval   interval,
  c_bytea      bytea,
  c_int_arr    integer[],
  c_text_arr   text[],
  c_enum       mood
);
