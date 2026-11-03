#!/usr/bin/env bash
# Recreates the sandb-test stack with fresh seed data.
set -euo pipefail
cd "$(dirname "$0")"

./down.sh
exec ./up.sh
