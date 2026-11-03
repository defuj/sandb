-- sandb test seed (MySQL 8.4 + MariaDB 11): users. Runs last.
-- Local test-only passwords (same as .env.test).
CREATE USER 'sandb_rw'@'%' IDENTIFIED BY 'sandb_rw_pw';
CREATE USER 'sandb_ro'@'%' IDENTIFIED BY 'sandb_ro_pw';
GRANT ALL PRIVILEGES ON shop.* TO 'sandb_rw'@'%';
GRANT ALL PRIVILEGES ON analytics.* TO 'sandb_rw'@'%';
GRANT SELECT, SHOW VIEW ON shop.* TO 'sandb_ro'@'%';
GRANT SELECT, SHOW VIEW ON analytics.* TO 'sandb_ro'@'%';
FLUSH PRIVILEGES;
