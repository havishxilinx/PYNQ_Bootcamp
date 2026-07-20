# GridMind on Real KV260 Hardware — Simulated-Camera Demo Guide

This is a real end-to-end run of the whole GridMind stack using two actual KV260 boards
(checked out from the systest board farm) as the two competing teams, instead of the
`demo_student_bot.py` simulator used elsewhere in this project. The only thing simulated
is the camera input (no physical grid is attached to these boards) — everything else
(real DPU/YOLO inference, the real P2P wire protocol, the real referee/scoring engine,
the real operator console) is exactly what runs on competition day.

## What you need before starting

- Two KV260 boards checked out via systest, each with a `tcpforward 9090 '10.10.70.1:9090'`
  session active (Jupyter password: `xilinx`). See `three-machine-demo-guide.md` /
  Confluence page `1771115077` for the checkout steps if starting fresh.
- **The board's system clock must show the real current date.** If a fresh systest KV260's
  clock is stuck around mid-2024 (a known, undocumented issue — nothing in Confluence
  mentions it), the training-material clone will fail with a TLS certificate error. Fix
  with `sudo timedatectl set-ntp true` or `sudo date -s "<today>"` in a notebook `%%bash`
  cell before doing anything else.
- The bootcamp training material cloned onto each board (via
  `reclone_IDT_training_materal.ipynb`, which restarts the `reclone_training.service`
  systemd unit) — this is what provides the real DPU model file (`tf_yolov3_voc.xmodel`)
  and sample images used below.

## Architecture for this specific test

```
   This machine (broker + master + both arenas)
        |
        |  P2P broker on 0.0.0.0:35050 (message relay only, no game logic)
        |
   +----+----------------------------+
   |                                 |
kv260-9 (Jupyter :9090)        kv260-6 (Jupyter :9090)
via tcpforward on 172.19.243.136   via tcpforward on 172.19.243.133
```

Both boards run the same `gridmind-referee` wire protocol as `demo_student_bot.py` does elsewhere
in this project (see `student-api-reference.html`) — just from a real PYNQ notebook instead of a
Python script, with real DPU/YOLO detection instead of a golden-answer-key lookup.

## Step 1 — Start the referee stack (this machine)

The p2p broker (`server.py`, `pynqp2p` library) is distributed separately
from this directory — **TODO(havish): fill in where organizers get it for
event day.** Once you have it:

```bash
cd <path-to-broker>
python3 server.py --host 0.0.0.0 --port 35050 --key demokey
```
**Watch:** the broker's own log lines — every `/send`/`/receive_all` call from any board, arena,
or master shows up here. This is the lowest-level view of the whole system; if something isn't
working, this is where you'll see whether a message ever arrived at all.

```bash
cd gridmind-referee
./target/debug/gridmind-referee master --server 127.0.0.1:35050 --key demokey \
  --id master-referee --config kv260_test_pools.json --web-port 38800
```
**Watch:** the master's own stdout. Right at startup it prints something like:
```
Team secrets for join_competition (config mode -- no registration screen to show these):
  kv260-9: <8-char secret>
  kv260-6: <8-char secret>
Waiting for operator to start the tournament (web console)...
```
**Copy both secrets down now** — you'll paste them into each board's notebook in Step 3. This
only prints in `--config` mode, since that path has no registration screen to show them on.

```bash
./target/debug/gridmind-referee arena --server 127.0.0.1:35050 --key demokey \
  --id arena-1-referee --master-id master-referee --arena-num 1
./target/debug/gridmind-referee arena --server 127.0.0.1:35050 --key demokey \
  --id arena-2-referee --master-id master-referee --arena-num 2
```
**Watch:** each arena's own stdout — silent until a match is actually assigned to it.

Then open in a browser:
- **Operator console**: `http://<this-machine-ip>:38800/operator`
- **Public scoreboard**: `http://<this-machine-ip>:38800/`

`kv260_test_pools.json` registers exactly 2 teams (`kv260-9`, `kv260-6`), both nominally in
pool 1 — with exactly 2 teams total, the tournament engine's two-team dry-run logic skips
pools entirely and goes straight to a single Grand-Final-style match (see `PROJECT_STATE.md`).
It also points `grid_id` at `kv260_test_grid.json`, a custom 3x5 (15-cell) grid using only the
two object classes we have real sample photos for on the boards: `dog` and `bottle`.

## Step 2 — Confirm the referee actually offered a match

**Watch the master's log** for a line like:
```
Next match for arena 1: kv260-9 vs kv260-6
Waiting for operator to record the puzzle-race winner and board MACs via the web console...
```
This tells you which arena (`arena-1-referee` or `arena-2-referee`) got the match — you'll need
this exact value for `REFEREE_ID` in both boards' notebooks in Step 3. **Also refresh the operator
console in your browser** — you should see a single "Ready" row for this match under the Grand
Final section (since it's a two-team dry run, not regular pool play).

## Step 3 — Configure and connect each board

Open each board's Jupyter (`http://172.19.243.136:9090` for kv260-9, `http://172.19.243.133:9090`
for kv260-6, password `xilinx`), open the uploaded `PYNQ_302-Referee_Match_Client-SIMULATED_CAMERA.ipynb`.

In the **Match Configuration** cell (section 2), set:
```python
SERVER = '<this-machine-ip>:35050'   # the broker
BROKER_KEY = 'demokey'
REFEREE_ID = 'arena-1-referee'       # or arena-2-referee -- whichever Step 2 showed
MASTER_ID = 'master-referee'
TEAM_NAME = 'kv260-9'                # or 'kv260-6' on the other board
TEAM_SECRET = '<the secret Step 1 printed for this team>'
GRID_ROWS = 3
GRID_COLS = 5
```

Run every cell **except** the Camera section (section 5) — a markdown cell right above it says
so explicitly. Instead, run the new **"6a. Simulated Camera Override"** cell further down: it
redefines `detect_position()` to run the exact same real YOLO/DPU inference on a pre-staged photo
(`img/irishterrier-696543.JPEG` for `dog`, `img/bottle.JPEG` for `bottle`) instead of a live camera
frame — genuinely real detection, just not pointed at a physical grid.

In the widget GUI (section 10), fill in the same values and click **Connect**.

**Watch:** the moment you click Connect, if `TEAM_SECRET` is filled in, the notebook sends
`join_competition` immediately — check the **operator console's match popup** (open it by
clicking the Ready row from Step 2): that team's MAC field should turn green and auto-fill with
the board's real MAC address within about a second. This is the actual Join Competition feature
from tonight's work, running against real hardware for the first time.

## Step 4 — Start the match

Once both boards show green/auto-filled MACs in the popup (or you've entered them manually as a
fallback), pick a puzzle-race winner and submit. **Watch:**
- **The scoreboard** (`http://<this-machine-ip>:38800/`) — the live grid appears, turn indicator,
  scores, streak, turn timer counting down from 120s.
- **Each board's notebook status panel** — "Your turn" / "Waiting" banners flip back and forth,
  the little grid table fills in with detected object names as positions get revealed.
- **The master's log** — one `[arena N, pool 0] X/7 pairs, scores: {...}` line per action.

Each board's `MatchClient` plays fully autonomously once you flip its Play Mode toggle on
(Wait Mode is the safety default — it stays connected but won't act on its own turn until you
switch it). On its turn: picks two positions, runs real DPU inference via `detect_position()` on
the pre-staged photo for each, compares its own two detections, and reports a `match`/`no_match`
claim — the referee validates that claim against `kv260_test_grid.json`'s real answer key and
scores accordingly (streak bonus/wrong-match penalty, plus this session's new response-time tier
based on how fast the report came back).

## Step 5 — Hints, if you want to see that path too

Type an object name (`dog` or `bottle`) into either board's "Hint object" box and click "Queue
Hint" **during the opponent's turn** (queuing after your own turn starts is too late for that
turn, by design). It's sent automatically at the start of that team's next turn, before the
auto-flip. **Watch:** the board's log panel shows the `hint_request` go out and a `hint_response`
riddle come back; the score drops by 1 immediately (hints cost a point, capped at 2 per match,
and only available if that team's current score is above 0).

## Step 6 — Game over

Once all 7 pairs are found, both boards' status panels show "Game over — winner: ...", the
scoreboard shows the same, and the master's log prints the final scores. Since this was a
two-team dry run, that's also the tournament champion — no further matches follow.

## Cleanup afterward

```bash
# On this machine:
pkill -f "gridmind-referee (master|arena)"
pkill -f "server.py --host 0.0.0.0 --port 35050"
```
Leave the boards' systest sessions and Jupyter alone unless you're done with them entirely —
tearing those down is a separate, explicit systest command, not part of this cleanup.
