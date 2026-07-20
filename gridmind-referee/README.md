# GridMind Referee

Server-side infrastructure for **GridMind**, the two-team AI card-matching
competition run at AMD PYNQ Bootcamp. This directory holds the tournament
orchestrator (Master), the per-match rules engine (Arena), the operator/
scoreboard web UI, and everything needed to run or rehearse an event.

Student-facing notebooks (vision detection, referee wire-protocol client)
live under `bootcamp_sessions/` elsewhere in this repo, not here.

## What's in this directory

| Path | What it is |
|---|---|
| `src/`, `Cargo.toml`, `Cargo.lock` | The Rust crate: Master (tournament orchestrator), Arena (per-match rules engine), and the axum-based web server (operator console, public scoreboard, per-arena UI). |
| `static/` | The three web UIs served by the crate: `operator.html`, `scoreboard.html`, `arena.html`. |
| `data/` | Content the referee loads at runtime: grid pool (`grids/`), riddle banks, MNIST digit assets, and `game_config.json` (see below). |
| `example_grid.json`, `example_pools_config.json`, `kv260_test_grid.json`, `kv260_test_pools.json` | Fixture files for local testing / `--config` mode. |
| `simulate_tournament.py`, `demo_student_bot.py` | Run a full tournament end-to-end with simulated boards — no hardware or student notebooks needed. See below. |
| `student-api-reference.html` | The wire-protocol reference (exact JSON message shapes) linked from the student guides. |
| `operators-guide.md` | Step-by-step guide for whoever runs the tournament from the operator console on event day. |
| `student-competition-guide.md`, `student-implementation-guide.md`, `student-rescue-modules.md` | What students need to know to build their board's client and compete. |
| `kv260-real-hardware-demo-guide.md`, `manual-demo-walkthrough.md`, `three-machine-demo-guide.md` | Rehearsal/demo guides for different setups (real KV260s, manual curl walkthrough, distributed multi-machine). |

## Building

```bash
cd gridmind-referee
cargo build --release
cargo test
```

## Tuning timing and scoring before an event

Turn timeout, the physical-flip allowance, puzzle-race/free-hint window
lengths, response-time scoring tiers, and the paid-hint cap/cost all live in
`data/game_config.json` — edit that file and restart the Master/Arena
processes to apply changes, no rebuild required. See "Tuning timing and
scoring before the event" in `operators-guide.md` for the full picture of
what each value controls.

## Running a full tournament with no hardware

```bash
cd gridmind-referee
cargo build
python3 simulate_tournament.py --broker-dir <path-to-p2p-broker> --teams 6
```

This starts the broker, Master, and both Arenas, registers the given number
of simulated teams, and drives every match to completion with
`demo_student_bot.py` standing in for real boards — useful for rehearsing
the operator console/scoreboard without needing any student hardware
connected. Pass `--no-start-services` instead if you already have the
broker/Master/Arenas running elsewhere and just want this script to drive
registration and matches.

**Note:** the p2p broker (`server.py`, `pynqp2p` library) that all of this
talks over is distributed separately, not part of this directory —
**TODO(havish): fill in where organizers/students get it for event day.**

## Where to start reading

- Running the event? Start with `operators-guide.md`.
- Rehearsing without hardware? `manual-demo-walkthrough.md` (curl, step by
  step) or `simulate_tournament.py` (fully automated).
- Building a team's competition entry? `student-competition-guide.md`, then
  `student-implementation-guide.md` for the technical breakdown.
