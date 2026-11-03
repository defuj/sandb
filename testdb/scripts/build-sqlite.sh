#!/usr/bin/env bash
# Builds testdb/sqlite/shop.db (seeded) and testdb/sqlite/empty.db. Only these two files are replaced.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v sqlite3 >/dev/null || { echo "sqlite3 CLI not found" >&2; exit 1; }
mkdir -p sqlite
rm -f sqlite/shop.db sqlite/empty.db
sqlite3 -bail sqlite/shop.db < seed/sqlite/shop.sql
# VACUUM writes the header, so empty.db is a valid database file with no tables.
sqlite3 sqlite/empty.db 'VACUUM;'
echo "built $(pwd)/sqlite/shop.db and $(pwd)/sqlite/empty.db"
