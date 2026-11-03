-- sandb test seed (PostgreSQL): roles, ownership, shop_staging. Runs last.
-- Local test-only passwords (same as .env.test).
CREATE ROLE sandb_rw LOGIN PASSWORD 'sandb_rw_pw';
CREATE ROLE sandb_ro LOGIN PASSWORD 'sandb_ro_pw';

ALTER DATABASE shop OWNER TO sandb_rw;
ALTER SCHEMA public OWNER TO sandb_rw;
ALTER SCHEMA analytics OWNER TO sandb_rw;
ALTER TYPE mood OWNER TO sandb_rw;

DO $$
DECLARE r record;
BEGIN
  FOR r IN SELECT n.nspname, c.relname, c.relkind FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
           WHERE n.nspname IN ('public', 'analytics') AND c.relkind IN ('r', 'v', 'm', 'S') LOOP
    IF r.relkind = 'S' THEN
      -- Sequences owned by a serial column move together with their table.
      CONTINUE WHEN EXISTS (SELECT 1 FROM pg_depend d JOIN pg_class s ON s.oid = d.objid
                            WHERE s.relname = r.relname AND d.deptype = 'a');
      EXECUTE format('ALTER SEQUENCE %I.%I OWNER TO sandb_rw', r.nspname, r.relname);
    ELSIF r.relkind = 'v' THEN
      EXECUTE format('ALTER VIEW %I.%I OWNER TO sandb_rw', r.nspname, r.relname);
    ELSE
      EXECUTE format('ALTER TABLE %I.%I OWNER TO sandb_rw', r.nspname, r.relname);
    END IF;
  END LOOP;
END $$;

-- PG <= 14 lets PUBLIC create objects in schema public. Revoke it so sandb_ro stays read-only on pg13.
REVOKE CREATE ON SCHEMA public FROM PUBLIC;

GRANT CONNECT ON DATABASE shop TO sandb_ro;
GRANT USAGE ON SCHEMA public, analytics TO sandb_ro;
GRANT SELECT ON ALL TABLES IN SCHEMA public, analytics TO sandb_ro;
ALTER DEFAULT PRIVILEGES FOR ROLE sandb_rw IN SCHEMA public, analytics GRANT SELECT ON TABLES TO sandb_ro;

-- shop_staging is a copy of shop. A template copy needs no open sessions on shop.
\connect postgres
CREATE DATABASE shop_staging TEMPLATE shop OWNER sandb_rw;
GRANT CONNECT ON DATABASE shop_staging TO sandb_ro;
