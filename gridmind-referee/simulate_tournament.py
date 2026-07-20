#!/usr/bin/env python3
"""
Simulates a full GridMind tournament end-to-end -- registration, pool play,
Grand Final, champion -- using demo_student_bot.py as stand-ins for real
boards. Lets you demo the referee's scoreboard/operator GUI without needing
any real hardware connected.

By default this also starts the broker, master, and both arenas itself. Use
--no-start-services if you already have them running and just want this
script to drive registration + matches. The p2p broker (server.py) is
distributed separately from this crate -- pass its directory with
--broker-dir (or set the GRIDMIND_BROKER_DIR env var).

Usage:
    python3 simulate_tournament.py --broker-dir /path/to/broker
    python3 simulate_tournament.py --broker-dir /path/to/broker --teams 6
    python3 simulate_tournament.py --no-start-services   # broker/master/arenas already running
"""
import argparse
import asyncio
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

import requests
import websockets

SCRIPT_DIR = Path(__file__).resolve().parent          # gridmind-referee/
REFEREE_DIR = SCRIPT_DIR
REFEREE_BIN = REFEREE_DIR / "target" / "debug" / "gridmind-referee"
DEMO_BOT = SCRIPT_DIR / "demo_student_bot.py"

LOG_DIR = SCRIPT_DIR / "simulate_logs"


def start_services(broker_dir, broker_port, web_port, key):
    LOG_DIR.mkdir(exist_ok=True)
    processes = []

    def spawn(name, cmd, cwd):
        log_path = LOG_DIR / f"{name}.log"
        log_file = open(log_path, "w")
        proc = subprocess.Popen(cmd, cwd=cwd, stdout=log_file, stderr=subprocess.STDOUT)
        processes.append((name, proc, log_file))
        print(f"Started {name} (pid {proc.pid}), log: {log_path}")
        return proc

    spawn("broker", ["python3", "server.py", "--host", "0.0.0.0", "--port", str(broker_port), "--key", key], broker_dir)
    time.sleep(1.5)
    spawn("master", [str(REFEREE_BIN), "master", "--server", f"127.0.0.1:{broker_port}", "--key", key,
                      "--id", "master-referee", "--web-port", str(web_port)], REFEREE_DIR)
    time.sleep(1.5)
    spawn("arena-1", [str(REFEREE_BIN), "arena", "--server", f"127.0.0.1:{broker_port}", "--key", key,
                       "--id", "arena-1-referee", "--master-id", "master-referee", "--arena-num", "1"], REFEREE_DIR)
    spawn("arena-2", [str(REFEREE_BIN), "arena", "--server", f"127.0.0.1:{broker_port}", "--key", key,
                       "--id", "arena-2-referee", "--master-id", "master-referee", "--arena-num", "2"], REFEREE_DIR)
    time.sleep(1.5)
    return processes


def api(web_port, method, path, **kwargs):
    url = f"http://127.0.0.1:{web_port}{path}"
    response = requests.request(method, url, timeout=10, **kwargs)
    return response


async def fetch_state(web_port):
    async with websockets.connect(f"ws://127.0.0.1:{web_port}/ws") as ws:
        return json.loads(await asyncio.wait_for(ws.recv(), timeout=10))


def get_state(web_port):
    return asyncio.run(fetch_state(web_port))


def register_teams(web_port, team_names):
    for name in team_names:
        response = api(web_port, "POST", "/api/register-team", json={"name": name, "students": [f"Student {name}"]})
        print(f"register {name}: {response.text}")
    print(api(web_port, "POST", "/api/close-registration").text)
    print(api(web_port, "POST", "/api/start-tournament").text)


_mac_counter = 0
_mac_by_team = {}


def mac_for(team_name):
    """A stable, valid MAC per team name, assigned once on first use (a
    plain incrementing counter -- team names hashed into a MAC octet can
    overflow 0xff and produce an invalid, too-long MAC string)."""
    global _mac_counter
    if team_name not in _mac_by_team:
        _mac_counter += 1
        _mac_by_team[team_name] = f"aa:00:00:00:00:{_mac_counter:02x}"
    return _mac_by_team[team_name]


class BotSupervisor:
    """Keeps a demo_student_bot.py process running for one team in one
    match, restarting it if it crashes (the 30s wait_for timeout under
    sandbox load is a known demo-harness fragility, not a real bug -- see
    PROJECT_STATE.md). Call stop() once the match this bot belongs to
    reports as complete."""

    def __init__(self, team, board_id, referee_id, grid_path, broker_addr, broker_key, flip_mode="double"):
        self.team = team
        self.board_id = board_id
        self.referee_id = referee_id
        self.grid_path = grid_path
        self.broker_addr = broker_addr
        self.broker_key = broker_key
        self.flip_mode = flip_mode
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._thread.start()

    def stop(self):
        self._stop.set()

    def _run(self):
        log_path = LOG_DIR / f"bot-{self.team}.log"
        while not self._stop.is_set():
            with open(log_path, "a") as log_file:
                log_file.write(f"\n--- (re)launching bot for {self.team} ---\n")
                log_file.flush()
                proc = subprocess.Popen(
                    ["python3", str(DEMO_BOT), "--server", self.broker_addr, "--key", self.broker_key,
                     "--team", self.team, "--id", self.board_id, "--referee-id", self.referee_id,
                     "--grid", str(self.grid_path), "--flip-mode", self.flip_mode],
                    stdout=log_file, stderr=subprocess.STDOUT,
                )
                while proc.poll() is None:
                    if self._stop.is_set():
                        proc.terminate()
                        return
                    time.sleep(0.5)
                if proc.returncode == 0 or self._stop.is_set():
                    return
            time.sleep(1)  # brief pause before reconnecting


def resolve_grid_path(grid_id):
    # grid_id from the schedule is relative to the referee's own cwd, e.g.
    # "data/grids/grid_1.json" -- resolve it against REFEREE_DIR.
    return REFEREE_DIR / grid_id


def submit_match(web_port, arena_num, team_a, team_b, grid_id, broker_addr, broker_key, flip_mode):
    """Non-blocking: submits the puzzle-race winner + MACs (unblocking the
    referee's prompt_and_assign, which is waiting on exactly this) and
    launches both bots in the background. Returns the two BotSupervisors so
    the caller's own polling loop can stop them once the match completes --
    this function must NOT block, since both arenas need to be driven from
    the same top-level loop concurrently."""
    referee_id = f"arena-{arena_num}-referee"
    grid_path = resolve_grid_path(grid_id)
    print(f"[arena {arena_num}] submitting match: {team_a} vs {team_b} (grid {grid_id})")

    bot_a = BotSupervisor(team_a, mac_for(team_a), referee_id, grid_path, broker_addr, broker_key, flip_mode)
    bot_b = BotSupervisor(team_b, mac_for(team_b), referee_id, grid_path, broker_addr, broker_key, flip_mode)
    bot_a.start()
    bot_b.start()

    response = api(web_port, "POST", "/api/start-match", json={
        "arena": arena_num, "winner": team_a,
        "team_a_mac": bot_a.board_id, "team_b_mac": bot_b.board_id,
        # demo_student_bot.py uses the older simple wire protocol and never
        # calls join_competition, so the join-gating check (added earlier
        # today) would otherwise reject every match here.
        "force": True,
    })
    print(f"[arena {arena_num}] start-match response: {response.status_code} {response.text}")
    return bot_a, bot_b


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--teams", type=int, default=4)
    parser.add_argument("--broker-dir", default=os.environ.get("GRIDMIND_BROKER_DIR"),
                         help="directory containing the p2p broker's server.py (or set GRIDMIND_BROKER_DIR); "
                              "required unless --no-start-services")
    parser.add_argument("--broker-port", type=int, default=35050)
    parser.add_argument("--web-port", type=int, default=38801)
    parser.add_argument("--key", default="demokey")
    parser.add_argument("--flip-mode", default="double", choices=["single", "double"])
    parser.add_argument("--no-start-services", action="store_true")
    parser.add_argument("--no-wait", action="store_true",
                         help="skip the pause before registration -- start the tournament immediately")
    args = parser.parse_args()
    sys.stdout.reconfigure(line_buffering=True)  # so progress shows up promptly even when piped to a file

    if not args.no_start_services:
        if not args.broker_dir:
            parser.error("--broker-dir (or GRIDMIND_BROKER_DIR) is required unless --no-start-services is set")
        (REFEREE_DIR / "data" / "tournament_state.json").unlink(missing_ok=True)
        start_services(Path(args.broker_dir), args.broker_port, args.web_port, args.key)
    LOG_DIR.mkdir(exist_ok=True)

    if not args.no_wait:
        print(f"\nServices are up. Scoreboard/operator GUI: http://127.0.0.1:{args.web_port}/")
        input("Press Enter when you're ready to register teams and start the tournament...")

    team_names = [f"team-{i + 1}" for i in range(args.teams)]
    print(f"Registering teams: {team_names}")
    register_teams(args.web_port, team_names)

    broker_addr = f"127.0.0.1:{args.broker_port}"

    # arena_num -> {'key': (team_a, team_b), 'bots': (BotSupervisor, BotSupervisor)}
    active = {1: None, 2: None}

    def maybe_submit(arena_num, team_a, team_b, grid_id):
        key = (team_a, team_b)
        if active[arena_num] is not None and active[arena_num]["key"] == key:
            return  # already submitted/running this exact matchup
        bots = submit_match(args.web_port, arena_num, team_a, team_b, grid_id, broker_addr, args.key, args.flip_mode)
        active[arena_num] = {"key": key, "bots": bots}

    def maybe_stop(arena_num, still_live_key):
        entry = active[arena_num]
        if entry is not None and entry["key"] != still_live_key:
            for bot in entry["bots"]:
                bot.stop()
            active[arena_num] = None

    while True:
        time.sleep(2)
        state = get_state(args.web_port)
        phase = state.get("phase")

        if phase == "champion":
            for entry in active.values():
                if entry:
                    for bot in entry["bots"]:
                        bot.stop()
            print(f"\nCHAMPION: {state['winner']} (pool1: {state['pool1_winner']}, pool2: {state['pool2_winner']})")
            return

        if phase == "grand_final":
            # By the time phase flips to grand_final the match is already
            # live -- bots were already launched during its pregame
            # ceremony above (correct grid_id resolved there). This is just
            # a safety net; maybe_submit no-ops since the key already
            # matches in the normal case.
            arena_num = state["arena_num"]
            live = state["arena"]
            key = (live["team_a"], live["team_b"])
            fallback_grid_id = state["pool1_schedule"][0]["grid_id"] if state.get("pool1_schedule") else "data/grids/grid_1.json"
            maybe_submit(arena_num, live["team_a"], live["team_b"], fallback_grid_id)
            maybe_stop(arena_num, key)
            continue

        if phase != "live_pool_play":
            continue

        for arena_num, schedule_key, pregame_key, live_key in (
            (1, "pool1_schedule", "arena1_pregame", "arena1"),
            (2, "pool2_schedule", "arena2_pregame", "arena2"),
        ):
            pregame = state.get(pregame_key)
            live = state.get(live_key)

            if pregame:
                # The referee is blocked waiting for exactly this submission
                # (puzzle winner + MACs) -- submit right away rather than
                # treating "pregame in progress" as "nothing to do". The
                # matching schedule entry (locked/ready) has the grid_id --
                # except the Grand Final, which never appears in either
                # pool's schedule; it deliberately reuses
                # pool1_schedule[0].grid_id (see master.rs / PROJECT_STATE.md).
                schedule_entry = next(
                    (e for e in state.get(schedule_key, [])
                     if {e.get("team_a"), e.get("team_b")} == {pregame["team_a"], pregame["team_b"]}),
                    None,
                )
                if schedule_entry:
                    grid_id = schedule_entry["grid_id"]
                elif state.get("pool1_schedule"):
                    grid_id = state["pool1_schedule"][0]["grid_id"]
                else:
                    grid_id = "data/grids/grid_1.json"
                maybe_submit(arena_num, pregame["team_a"], pregame["team_b"], grid_id)
                continue

            if live:
                maybe_stop(arena_num, (live["team_a"], live["team_b"]))
                continue

            # Neither pregame nor live -- either idle (nothing scheduled
            # yet) or a match just finished. Stop any bots left over from a
            # completed match, and start the next "ready" one if there is one.
            maybe_stop(arena_num, None)
            ready = next((e for e in state.get(schedule_key, []) if e.get("status") == "ready"), None)
            if ready:
                maybe_submit(arena_num, ready["team_a"], ready["team_b"], ready["grid_id"])


if __name__ == "__main__":
    main()
