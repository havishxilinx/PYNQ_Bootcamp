#!/usr/bin/env bash
# Self-serve launch for the "master side" machine: broker relay + Master.
# Run launch-arena.sh on a separate (typically GPU) machine, passing this
# machine's IP (printed below) as its master-ip argument. Run
# setup-server.sh once beforehand on a fresh machine (installs the
# broker's one Python dependency).
#
# Always uses the same ports (override via env if you really need to):
#   broker=35050  master web=38800
# Any process already bound to those ports is killed before starting fresh.
#
# Usage:
#   ./launch-master.sh
#   KEY=mykey CONFIG=./data/pools_2teams.json ./launch-master.sh
#
# Ctrl+C (or closing this terminal) stops both processes it started.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BROKER_DIR="$ROOT/broker"
LOG_DIR="/tmp/gridmind-master-$(date +%Y%m%d 2>/dev/null || echo run)"

BROKER_PORT="${BROKER_PORT:-35050}"
MASTER_WEB_PORT="${MASTER_WEB_PORT:-38800}"
KEY="${KEY:-bootcamp}"
CONFIG="${CONFIG:-$ROOT/data/pools_2teams.json}"

# ── IP auto-detection (for display -- pass this to launch-arena.sh) ────────
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

# ── Port cleanup ─────────────────────────────────────────────────────────────
free_port() {
    local port="$1"
    if command -v fuser >/dev/null 2>&1; then
        fuser -k "${port}/tcp" >/dev/null 2>&1 || true
    elif command -v lsof >/dev/null 2>&1; then
        lsof -ti tcp:"$port" 2>/dev/null | xargs -r kill -9 2>/dev/null || true
    fi
}
echo "=== Cleaning up ports (broker=$BROKER_PORT master-web=$MASTER_WEB_PORT) ==="
for p in "$BROKER_PORT" "$MASTER_WEB_PORT"; do
    free_port "$p"
done
sleep 0.5

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
    echo "=== Stopping broker and master ==="
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

# ── Broker ───────────────────────────────────────────────────────────────────
echo "=== [1/2] Starting broker on 0.0.0.0:$BROKER_PORT ==="
BROKER_PYTHON="python3"
if [ -x "$BROKER_DIR/venv/bin/python" ]; then
    BROKER_PYTHON="$BROKER_DIR/venv/bin/python"
fi
( cd "$BROKER_DIR" && exec "$BROKER_PYTHON" server.py --host 0.0.0.0 --port "$BROKER_PORT" --key "$KEY" ) \
    > "$LOG_DIR/broker.log" 2>&1 &
PIDS+=("$!")
for i in $(seq 1 20); do
    if curl -sf -X POST "http://127.0.0.1:$BROKER_PORT/ping" -d "key=$KEY&id=launch-check" > /dev/null 2>&1; then
        echo "broker is up"
        break
    fi
    [ "$i" -eq 20 ] && { echo "error: broker failed to start, see $LOG_DIR/broker.log" >&2; exit 1; }
    sleep 0.5
done

# ── Master ───────────────────────────────────────────────────────────────────
echo "=== [2/2] Starting Master on web port $MASTER_WEB_PORT ==="
( cd "$ROOT" && exec "$REFEREE_BIN" master \
    --server "$SERVER_IP:$BROKER_PORT" --key "$KEY" --id master-referee \
    --web-port "$MASTER_WEB_PORT" --config "$CONFIG" ) \
    > "$LOG_DIR/master.log" 2>&1 &
PIDS+=("$!")

echo
echo "================================================================"
echo "Master side is up."
echo "  Operator console : http://$SERVER_IP:$MASTER_WEB_PORT/operator"
echo "  Scoreboard       : http://$SERVER_IP:$MASTER_WEB_PORT/"
echo "  Broker           : $SERVER_IP:$BROKER_PORT (key: $KEY)"
echo "  Logs             : $LOG_DIR/*.log"
echo
echo "On the arena/GPU machine, run:"
echo "  ./launch-arena.sh $SERVER_IP"
echo
echo "Press Ctrl+C to stop everything on this machine."
echo "================================================================"

wait
