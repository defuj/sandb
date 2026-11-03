#!/usr/bin/env bash
# Stops and removes only the sandb-test stack. Data is on tmpfs, so nothing is left behind.
set -euo pipefail
cd "$(dirname "$0")/.."

docker compose -p sandb-test -f compose.yaml --env-file .env.test down --remove-orphans
