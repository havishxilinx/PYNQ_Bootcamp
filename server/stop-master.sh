#!/usr/bin/env bash
# Frees the master-side ports without launching anything -- for when
# launch-master.sh's terminal was killed before its cleanup trap could run.
set -euo pipefail

BROKER_PORT="${BROKER_PORT:-35050}"
MASTER_WEB_PORT="${MASTER_WEB_PORT:-38800}"

free_port() {
    local port="$1"
    if command -v fuser >/dev/null 2>&1; then
        fuser -k "${port}/tcp" >/dev/null 2>&1 || true
    elif command -v lsof >/dev/null 2>&1; then
        lsof -ti tcp:"$port" 2>/dev/null | xargs -r kill -9 2>/dev/null || true
    fi
}

for p in "$BROKER_PORT" "$MASTER_WEB_PORT"; do
    echo "Freeing port $p..."
    free_port "$p"
done
echo "Done."
