#!/usr/bin/env bash
# Frees the arena-side ports without launching anything -- for when
# launch-arena.sh's terminal was killed before its cleanup trap could run.
set -euo pipefail

GENESIS_PORT="${GENESIS_PORT:-9002}"
GENESIS_STREAM_PORT="${GENESIS_STREAM_PORT:-9005}"

free_port() {
    local port="$1"
    if command -v fuser >/dev/null 2>&1; then
        fuser -k "${port}/tcp" >/dev/null 2>&1 || true
    elif command -v lsof >/dev/null 2>&1; then
        lsof -ti tcp:"$port" 2>/dev/null | xargs -r kill -9 2>/dev/null || true
    fi
}

for p in "$GENESIS_PORT" "$GENESIS_STREAM_PORT"; do
    echo "Freeing port $p..."
    free_port "$p"
done
echo "Done."
