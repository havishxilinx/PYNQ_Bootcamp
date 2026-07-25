#!/usr/bin/env bash
# Self-serve launch for the "arena side" machine (typically the GPU box):
# Genesis simulation + both Arena referees (arena-num is always 1 and 2 --
# gridmind-referee's schedule logic is hardcoded to exactly 2 arenas, and
# pool 1 always plays on arena 1 / pool 2 on arena 2). Run launch-master.sh
# on a separate machine first and pass its IP here. Set up Genesis first
# (see genesis/QUICKSTART.md -- not done by setup-server.sh) before running
# this.
#
# Always uses the same ports (override via env if you really need to):
#   genesis=9002  genesis-stream=9005
# Any process already bound to those ports is killed before starting fresh.
#
# Usage:
#   ./launch-arena.sh <master-ip>
#   MASTER_IP=192.168.1.10 ./launch-arena.sh
#   HIDE_GENESIS_VIDEO=false ./launch-arena.sh 192.168.1.10   # show Genesis video in arena.html
#
# Ctrl+C (or closing this terminal) stops every process it started.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GENESIS_DIR="${GENESIS_DIR:-$ROOT/genesis}"
LOG_DIR="/tmp/gridmind-arena-$(date +%Y%m%d 2>/dev/null || echo run)"

MASTER_IP="${MASTER_IP:-${1:-}}"
if [ -z "$MASTER_IP" ]; then
    echo "usage: $0 <master-ip>   (or set MASTER_IP=x.x.x.x)" >&2
    echo "        this is the IP launch-master.sh printed on the other machine" >&2
    exit 1
fi

BROKER_PORT="${BROKER_PORT:-35050}"
GENESIS_PORT="${GENESIS_PORT:-9002}"
GENESIS_STREAM_PORT="${GENESIS_STREAM_PORT:-9005}"
KEY="${KEY:-bootcamp}"
HIDE_GENESIS_VIDEO="${HIDE_GENESIS_VIDEO:-true}"
GENESIS_BACKEND="${GENESIS_BACKEND:-gpu}"

# ── IP auto-detection (for genesis-url -- must be reachable by the operator's/students' browser) ─
detect_ip() {
    local ip=""
    if command -v ip >/dev/null 2>&1; then
        ip=$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -1)
    fi
    if [ -z "$ip" ] && command -v hostname >/dev/null 2>&1; then
        ip=$(hostname -I 2>/dev/null | awk '{print $1}')
    fi
    echo "$ip"
}
SERVER_IP="${SERVER_IP:-$(detect_ip)}"
if [ -z "$SERVER_IP" ]; then
    echo "error: could not auto-detect this machine's IP -- set SERVER_IP=x.x.x.x explicitly" >&2
    exit 1
fi

# ── Port cleanup (Genesis only -- arena processes are P2P clients with no listening port of their own) ─
free_port() {
    local port="$1"
    if command -v fuser >/dev/null 2>&1; then
        fuser -k "${port}/tcp" >/dev/null 2>&1 || true
    elif command -v lsof >/dev/null 2>&1; then
        lsof -ti tcp:"$port" 2>/dev/null | xargs -r kill -9 2>/dev/null || true
    fi
}
echo "=== Cleaning up ports (genesis=$GENESIS_PORT stream=$GENESIS_STREAM_PORT) ==="
for p in "$GENESIS_PORT" "$GENESIS_STREAM_PORT"; do
    free_port "$p"
done
sleep 0.5

if [ ! -f "$GENESIS_DIR/genesis_server/server.py" ]; then
    echo "error: $GENESIS_DIR/genesis_server/server.py not found -- set GENESIS_DIR=/path/to/genesis to override" >&2
    exit 1
fi
GENESIS_PYTHON="python3"
if [ -x "$GENESIS_DIR/venv/bin/python" ]; then
    GENESIS_PYTHON="$GENESIS_DIR/venv/bin/python"
elif [ -x "$GENESIS_DIR/.venv/bin/python" ]; then
    GENESIS_PYTHON="$GENESIS_DIR/.venv/bin/python"
else
    echo "warning: no venv found under $GENESIS_DIR -- falling back to system python3 (see genesis/QUICKSTART.md)" >&2
fi

# ── Genesis admin password ──────────────────────────────────────────────────
# No fixed default -- reuse one securely-generated value for both sides
# (genesis_server's own GENESIS_ADMIN_PASSWORD and each arena's
# --genesis-admin-password) unless the operator already set one.
if [ -z "${GENESIS_ADMIN_PASSWORD:-}" ]; then
    if command -v openssl >/dev/null 2>&1; then
        GENESIS_ADMIN_PASSWORD="$(openssl rand -hex 12)"
    else
        GENESIS_ADMIN_PASSWORD="$(tr -dc 'a-zA-Z0-9' < /dev/urandom | head -c 24)"
    fi
    echo "=== Generated Genesis admin password for this run: $GENESIS_ADMIN_PASSWORD ==="
fi

REFEREE_BIN="$ROOT/gridmind-referee"
if [ ! -x "$REFEREE_BIN" ]; then
    echo "error: $REFEREE_BIN not found or not executable -- run setup-server.sh first" >&2
    exit 1
fi

mkdir -p "$LOG_DIR"
echo "=== Logs: $LOG_DIR ==="

PIDS=()
cleanup() {
    echo
    echo "=== Stopping Genesis and both arenas ==="
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

# ── Genesis ──────────────────────────────────────────────────────────────────
echo "=== [1/2] Starting Genesis on :$GENESIS_PORT (stream :$GENESIS_STREAM_PORT) ==="
( cd "$GENESIS_DIR" && exec env \
    GENESIS_SHOW_VIEWER=true \
    PYTHONUNBUFFERED=1 \
    GENESIS_BACKEND="$GENESIS_BACKEND" \
    GENESIS_PORT="$GENESIS_PORT" \
    GENESIS_STREAM_PORT="$GENESIS_STREAM_PORT" \
    GENESIS_ADMIN_PASSWORD="$GENESIS_ADMIN_PASSWORD" \
    "$GENESIS_PYTHON" -u -m genesis_server.server ) \
    > "$LOG_DIR/genesis.log" 2>&1 &
PIDS+=("$!")

# ── Arenas (always exactly 2; pool 1 -> arena 1, pool 2 -> arena 2) ─────────
HIDE_FLAG=()
if [ "$HIDE_GENESIS_VIDEO" = "true" ]; then
    HIDE_FLAG=(--hide-genesis-video)
fi
echo "=== [2/2] Starting Arena 1 and Arena 2 (master at $MASTER_IP) ==="
for n in 1 2; do
    ( cd "$ROOT" && exec "$REFEREE_BIN" arena \
        --server "$MASTER_IP:$BROKER_PORT" --master-id master-referee --arena-num "$n" \
        --genesis-url "http://$SERVER_IP:$GENESIS_PORT" \
        --genesis-admin-password "$GENESIS_ADMIN_PASSWORD" \
        --genesis-stream-port "$GENESIS_STREAM_PORT" \
        "${HIDE_FLAG[@]}" \
        --id "arena-${n}-referee" --key "$KEY" ) \
        > "$LOG_DIR/arena${n}.log" 2>&1 &
    PIDS+=("$!")
done

echo
echo "================================================================"
echo "Arena side is up, connected to master at $MASTER_IP:$BROKER_PORT."
echo "  Genesis : http://$SERVER_IP:$GENESIS_PORT (admin password: $GENESIS_ADMIN_PASSWORD)"
echo "  Logs    : $LOG_DIR/*.log"
echo "Press Ctrl+C to stop everything on this machine."
echo "================================================================"

wait
