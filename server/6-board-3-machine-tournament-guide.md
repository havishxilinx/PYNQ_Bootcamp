# GridMind — 6-Board, 3-Machine Tournament Guide

A complete, exact runbook for this specific deployment shape:

- **6 PYNQ boards** (6 teams, split 3/3 into two pools)
- **3 server machines**: Machine 1 runs the broker + Master; Machine 2 runs Arena 1 + its own Genesis server; Machine 3 runs Arena 2 + its own Genesis server
- **10 grid files** in the live-registration grid pool

Every step below reflects the current referee behavior, including: the pregame ceremony now waits for both teams to actually join before sending anything timed (registration can happen days before the tournament itself), the operator's manual-join/resend/restart recovery controls, and free hints delivered as plain text (no QR decoding required on either side).

## Architecture

```
                         Machine 1 (broker + Master)
                                    |
                    P2P broker on 0.0.0.0:35050
                    Master web UI on :38800
                    (operator console, scoreboard, arena views)
                                    |
        +---------------------------+---------------------------+
        |                                                       |
   Machine 2                                              Machine 3
   Arena 1 (arena-1-referee)                          Arena 2 (arena-2-referee)
   Genesis 1: API :9002, stream :8080                 Genesis 2: API :9002, stream :8080
        |                                                       |
   Team A's board  <---- shared physical grid ---->  Team D's board
   Team B's board       (Arena 1's table)             Team E's board
   Team C's board                                     Team F's board
```

Each arena is a **shared physical table** two teams play at simultaneously — one human referee per arena physically flips cards on request; the same physical grid is what both connected boards' cameras look at. Six boards total, three per pool, matches assigned two at a time (one per arena) so the tournament runs in parallel across both arenas.

**Network requirement:** all 3 machines and all 6 boards must be able to reach Machine 1's IP on port 35050 (broker) and 38800 (web UI). Each board additionally needs to reach whichever arena's Genesis machine it's assigned to, on ports 9002 and 8080.

## Part 1 — Server setup

### Machine 1 — Broker + Master

```bash
git clone git@github.com-personal:havishxilinx/PYNQ_Bootcamp.git gridmind-tournament
cd gridmind-tournament
git checkout gridmind-package
cd server
./setup-server.sh
```

Prepare the grid pool (see Part 2) before starting the Master.

Start the broker:
```bash
cd broker
python3 server.py --host 0.0.0.0 --port 35050 --key <pick-a-real-event-key>
```

Start the Master (live registration — no `--config`):
```bash
cd ..
./gridmind-referee master --server 127.0.0.1:35050 --key <event-key> \
  --id master-referee --web-port 38800
```

Open on this machine (and share the IP with the projector / operator laptop):
- **Operator console:** `http://<machine-1-ip>:38800/operator`
- **Public scoreboard:** `http://<machine-1-ip>:38800/`

### Machine 2 — Arena 1 + Genesis 1

Get Machine 2's own LAN IP first (`ip -4 addr show`, call it `<machine-2-ip>`) — this matters, see the warning below.

**Start Genesis 1** (bundled in the package at `server/genesis/` — copy the whole `server/` folder to this machine same as any other server-role machine):
```bash
cd server/genesis
python3 -m venv venv && source venv/bin/activate && pip install -e .
export GENESIS_PORT=9002
export GENESIS_STREAM_PORT=8080
export GENESIS_BACKEND=cpu       # or amdgpu if this machine has a working ROCm torch install
export GENESIS_SHOW_VIEWER=false # disables Genesis's own native GUI window, not the web stream
export GENESIS_ADMIN_PASSWORD=admin123
python scripts/run_server.py
```

**Start Arena 1**, from the copied `server/` package folder on this machine:
```bash
./gridmind-referee arena --server <machine-1-ip>:35050 --key <event-key> \
  --id arena-1-referee --master-id master-referee --arena-num 1 \
  --genesis-url http://<machine-2-ip>:9002 --genesis-admin-password admin123 \
  --genesis-stream-port 8080
```

**Important:** use `<machine-2-ip>`, not `127.0.0.1`, for `--genesis-url` even though Genesis is on this same machine — that exact string gets forwarded to student boards (for their own `pynqsim` connection) and used to build the browser-facing video stream URL. Loopback would be unreachable from anywhere else.

Open on (or near) this machine: `http://<machine-1-ip>:38800/arena?arena=1` — the live score/timer/video view for this specific table.

### Machine 3 — Arena 2 + Genesis 2

Identical to Machine 2, with `--arena-num 2` / `--id arena-2-referee`, using Machine 3's own IP for `--genesis-url`:
```bash
./gridmind-referee arena --server <machine-1-ip>:35050 --key <event-key> \
  --id arena-2-referee --master-id master-referee --arena-num 2 \
  --genesis-url http://<machine-3-ip>:9002 --genesis-admin-password admin123 \
  --genesis-stream-port 8080
```
Open `http://<machine-1-ip>:38800/arena?arena=2` on/near this machine.

Genesis is entirely optional and cosmetic on both machines — skip it (omit `--genesis-*` flags) if you don't need the simulated-arm visual; the real match plays identically either way.

### Genesis configuration — who needs what, exactly

| Component | What it needs | How it gets it |
|---|---|---|
| **Genesis server** | The 5 env vars above (`GENESIS_PORT`, `GENESIS_STREAM_PORT`, `GENESIS_BACKEND`, `GENESIS_SHOW_VIEWER`, `GENESIS_ADMIN_PASSWORD`) | You set them before `run_server.py` |
| **Master** | **Nothing.** No `--genesis-*` flag exists on `gridmind-referee master` at all. | N/A |
| **Arena** | `--genesis-url`, `--genesis-admin-password` (must match that Genesis server's `GENESIS_ADMIN_PASSWORD`), `--genesis-stream-port` (must match that Genesis server's `GENESIS_STREAM_PORT`) | You pass them on the command line, once, per arena |
| **Student notebook / client** | **Nothing to type in.** No Genesis URL or team ID field exists in Match Configuration. | Arrives automatically inside the `game_start` message (`genesis_team_id`, `genesis_url`) the moment the match starts — the notebook connects to Genesis on its own from there. The only thing a student does is optionally `pip install pynqsim` (via `setup-client.sh`) so that automatic connection succeeds; without it, the arm animation is silently skipped and the real match is unaffected. |

If a board's console never shows a `🤖 Genesis: ...` badge during a match, check the *Arena's* `--genesis-url`/password/port first — that's the only place Genesis is actually configured.

## Part 2 — Grid pool (10 files)

On **Machine 1**, in `server/data/grids/`, place exactly the grids you want in play for this event — clear out anything left from earlier testing first:
```bash
cd server/data/grids
rm -f *.json   # start clean -- a leftover test fixture can otherwise get drawn for a real match
```
Add 10 files (`grid_01.json` .. `grid_10.json`), each shaped like:
```json
{"positions": {"A1": "dog", "A2": "person", "A3": "bottle", ...}}
```
Use an even cell count matching your physical board (e.g. 30 cells / 15 pairs for a 5×6 board), object classes your teams' YOLO models can actually detect. With 6 teams there are 7 total matches (see Part 4) — 10 grids means most matches get a distinct one, with only occasional repeats (grids are drawn randomly, with replacement, per match).

## Part 3 — Client setup (6 boards)

Copy `client/` to each board, then on each board:
```bash
cd client
./setup-client.sh
```
Answer `y` to installing `pynqsim` only for boards on an arena that has Genesis configured.

**Pick a notebook per team**, from `client/notebooks/`:
- `PYNQ_302-Referee_Match_Client.ipynb` — all 4 detection approaches, switchable via dropdown. Good for a team still comparing approaches.
- `PYNQ_302-<approach>-<Red|Blue>.ipynb` — one approach fixed (`yolo_full_frame`, `yolo_grid_crops`, `aruco_border`, or `aruco_per_card`), simpler stage-tracker UI instead of free-form status text. The `Red`/`Blue` in the filename is just a starting default for `TEAM_NAME` — **every team edits this to their actual registered name** regardless of which file they use; it has no connection to which physical arena or Genesis team-color they end up assigned.

Run every cell through **Match Configuration** and stop there — don't fill in `REFEREE_ID` yet (you don't have it until your match is actually assigned in Part 5).

## Part 4 — Registration

In the operator console (Registration view): click **Register Team** six times, once per team, entering each team's name and student names. Teams auto-split 3/3 into Pool 1 / Pool 2. Each team gets an **8-character secret** shown right there — write it down or have the team write it down; they'll paste it into `TEAM_SECRET` in their notebook.

Round-robin within a 3-team pool is 3 matches; two pools means **6 pool matches**, plus **1 Grand Final** between the pool winners — **7 matches total**.

Once all 6 are in: click **Close Registration** (one-way — builds both pools' schedules), then **Start Tournament**. The console switches to the Schedule view.

This can all happen well before the tournament itself runs — team formation and registration one day, students building their client the next, the actual matches on a later day. Nothing about registration requires any board to be connected yet.

## Part 5 — Running one complete game, step by step

This is the exact sequence for a single scheduled match, with who does what marked clearly:

**🤖 automatic** — the referee does this on its own
**🎛️ operator** — a click in the web console
**🖱️ team** — a click in that team's notebook
**✋ physical** — a human's hands on the actual cards

---

**1. Match becomes Ready** 🤖
A row in the Schedule view turns `Ready`. Above that arena's admin panel, a new panel appears: *"Waiting for both teams to connect"* with ⏳ next to each team name. **Nothing else happens yet** — no riddle, no timer running.

**2. Teams connect** 🖱️
Each of the two teams for this match: fill in `SERVER`, `BROKER_KEY`, `MASTER_ID`, `TEAM_NAME` (their real name), `TEAM_SECRET` (from Part 4) in Match Configuration (leave `REFEREE_ID` blank for now), then click **Connect** in the widget GUI. This self-reports their board's MAC.

**3. Operator watches both connect** 🎛️ (only if needed)
The pregame panel updates ⏳→✅ per team automatically as they connect (usually within ~1 second of Connect). If a team's board can't self-report for some reason, the operator types that team's name and MAC into the same panel and clicks **Mark as Joined** — this unblocks the ceremony identically to a real self-report.

**4. Riddle fires — automatically, the instant both show ✅** 🤖
Both teams get the real puzzle-race riddle over the wire. The public scoreboard and both arena views show a live countdown (120s by default) with the riddle text. Each notebook's built-in LLM immediately tries to solve it and prints its guess to that board's own log — purely a speed aid, it doesn't submit anything to the referee.

**5. Someone answers, out loud** ✋
**Judging is entirely manual, always.** Whichever team's human says the correct answer first, tell the operator — there's no automatic check. If nobody gets it before the timer runs out, decide however your event's rules say to (a coin flip is fine). If a team says they never got the riddle, use **Resend** in the pregame panel; if something's genuinely stuck, **Restart** picks a fresh riddle and resets the timer.

**6. Operator submits the winner** 🎛️
Click the Ready row to open the match-assign popup (MACs should already show green/auto-filled). Pick the puzzle-race winner, submit. This is what actually calls `/api/start-match`.

**7. Free hint — automatically, right after submission** 🤖
Both teams get a shared, non-competitive hint (a real object on this match's actual grid, plus a rough on-board quadrant, as plain text fragments assembled automatically) — another live countdown shows on the scoreboard (60s by default). No operator or team action needed; the match starts the instant this window ends.

**8. Match starts** 🤖 + 🖱️
Both boards receive `game_start` (team list, and Genesis connection info if configured). **Each team should already have clicked Start** during steps 2–7 (don't wait for game_start to do this):
- **Auto control mode**: clicking **Start** immediately puts the client in Play Mode too — fully autonomous from here, no more clicks for the rest of the match.
- **Manual control mode**: clicking **Start** leaves it in Wait Mode; the team must also flip the **Play Mode** toggle themselves once ready to let it act.

**9. Turn loop — repeats until all pairs are found**

*On the active team's turn* (stage tracker: Flip → Waiting → Detecting → Comparing → Result), all 🤖 automatic except one physical step:
- Client picks two positions (a known matching pair if it has one in memory, otherwise explores) and sends the flip request.
- The referee validates it's actually their turn and broadcasts the reveal to **both** boards.
- ✋ **Physical**: the instant a reveal is broadcast, both notebooks print *"FLIP THE PHYSICAL CARD AT \<position\> NOW"* — the arena's human referee physically flips that card face-up. There's a ~20-second allowance built into the scoring before either board's camera actually looks.
- Both boards independently run their own vision detection on the revealed card(s) — the active team's client compares its own two detections and submits a match/no_match claim; the referee validates that claim against the real grid and updates the score.

*On the other team's turn* (stage tracker: Waiting → Detecting → Logging) — same physical flip happens (one human referee per arena, not per team), and the *waiting* team's board also runs its own detection on what gets revealed, purely to keep its own memory in sync for its next turn. It does nothing else and sends nothing to the referee.

*Either team, either turn*: **Queue Hint** — a team can type an object name any time during the *opponent's* turn; it's requested automatically at the very start of their own next turn (costs 1 point, capped at 2 uses per match, only available above 0 score — silently ignored outside those conditions, no error shown).

**10. Game over** 🤖
Once all pairs are found, both boards' panels and the scoreboard show the winner and final scores. That arena resets to idle, ready for its next assignment.

## Part 6 — Repeat, then Grand Final, then Champion

Repeat Part 5 for the remaining 5 pool matches (both arenas can run one match each at the same time — this is why the schedule assigns matches to both arenas in parallel rather than one at a time). Once both pools finish all 3 of their matches, a Grand Final row appears automatically between the two pool winners — run it exactly like Part 5. Once it finishes, the console and scoreboard switch to the Champion screen.

**Known quirk:** the Master doesn't exit after a Champion is declared (the trophy screen is meant to stay up on the projector). Ctrl+C and restart it for a second tournament that day.

## Cleanup
```bash
# Machine 1:
pkill -f "gridmind-referee master"
pkill -f "server.py --host 0.0.0.0 --port 35050"
# Machines 2 and 3:
pkill -f "gridmind-referee arena"
pkill -f "scripts/run_server.py"   # Genesis, if used
```
