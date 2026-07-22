# GridMind Deployment Package (v4)

## What's new in v4

- **New: `6-board-3-machine-tournament-guide.md`.** A complete, exact
  runbook for a real 6-team event split across 3 machines (Master+broker,
  and one Arena+Genesis pair per machine) — covers server setup, a 10-file
  grid pool, client setup, and an exact step-by-step walkthrough of one
  full match with who does what marked (automatic / operator click / team
  click / physical hands-on-cards).
- **The pregame ceremony now waits for both teams to actually join.**
  Previously the puzzle-race riddle (and its countdown) fired the instant
  a match became `Ready` in the schedule — fine for a same-day event, but
  wrong for the realistic case of registration happening well before the
  tournament itself runs (team formation one day, building a client the
  next, the tournament later). The referee now shows a "waiting for both
  teams to connect" state and holds everything until both teams' MACs are
  known — via `join_competition` or a new operator **Mark as Joined**
  manual-entry fallback (`/api/manual-join`) for a board that can't
  self-report.
- **Free hints are now plain text, not QR-encoded images.** Removes an
  unnecessary image encode/decode round-trip on both ends — one less thing
  that could silently fail with no error (which is exactly what happened
  in a real hardware notebook missing its QR decoder). No functional loss:
  the hint was never meant to be a vision challenge, only the paid hint's
  row/col digit images are.
- **The Genesis simulated-arm server source is now bundled** at
  `server/genesis/` (previously "not part of this package" — you had to
  get it separately). Its own Python dependencies still need a manual,
  machine-specific install (GPU backend choice), see
  `server/genesis/QUICKSTART.md`.
- Everything else (comms resilience, deterministic/persisted team
  secrets, Resend/Restart controls, the 8 fixed-approach notebooks) is
  unchanged from v3.

## What was new in v3

- **Communication resilience.** A single failed broker send used to crash
  the whole Arena process mid-match, leaving both boards waiting on a
  `card_revealed` that would never arrive. The referee now retries
  transient send/receive failures, keeps delivering the rest of a message
  batch after one recipient fails, and survives a match-ending
  communication error by moving on to the next assignment instead of
  dying for the rest of the tournament.
- **Team secrets no longer silently expire on restart.** `--config` mode
  now derives secrets deterministically from the team name (same name,
  same secret, every restart), and a Master restart mid-registration
  restores already-registered teams' secrets instead of forgetting them.
  Previously either case could silently invalidate a board's
  already-typed-in `TEAM_SECRET` with no error anywhere.
- **Pregame Resend/Restart controls.** The operator console now shows a
  live pregame ceremony panel (previously nothing was shown there at all)
  with Resend/Restart buttons for the puzzle-race riddle or free hint — a
  team that joins even slightly late no longer permanently misses the
  riddle, and there's now a manual recovery path either way.
- **8 new fixed-approach match-client notebooks**
  (`PYNQ_302-<approach>-<Red|Blue>.ipynb`, one per detection approach x
  team color) replace the old plain `-Red.ipynb`/`-Blue.ipynb` pair — no
  approach-switching dropdown, and a fixed stage tracker (Join -> Riddle
  -> Free Hint -> Play/Wait sub-stages) instead of free-form status text.
  The original all-in-one `PYNQ_302-Referee_Match_Client.ipynb` is
  unchanged and still useful for comparing approaches on one board.
- Everything else is unchanged from v2 — same broker, same client
  libraries. `server/gridmind-referee` (rebuilt), `server/operators-guide.md`,
  `client/student-competition-guide.md`, and the notebook set differ.

---

Everything needed to run a real GridMind tournament, split into two
self-contained folders you copy to different machines:

- **`server/`** — copy to whichever machine(s) run the p2p broker, the
  Master (tournament orchestrator), and the Arenas (per-match rules
  engine). Can be one machine or split across several (see
  `three-machine-demo-guide.md` inside `server/`).
- **`client/`** — copy to each student PYNQ board.

No build step for the referee itself — `server/gridmind-referee` is a
prebuilt binary. See "What still needs installing" below for the couple of
things that genuinely can't be pre-packaged.

## Quick start

**On each server-role machine:**
```bash
cd server
./setup-server.sh        # chmod +x the binary, create broker/venv, install the broker's one dependency into it
```
Then follow `server/operators-guide.md` (or `server/manual-demo-walkthrough.md`
for a scripted curl-based rehearsal) to actually start the broker/Master/Arenas.

**On each PYNQ board** (after uploading `client/` via Jupyter's file browser,
or `scp`, and extracting it):
```bash
cd client
./setup-client.sh         # places pynqp2p_pkg where the notebook expects it
```
Then open one of `client/notebooks/*.ipynb` in Jupyter, fill in the Match
Configuration cell, and run it top to bottom. Full rules and wire protocol:
`client/student-competition-guide.md`.

## What's in `server/`

| Path | What it is |
|---|---|
| `gridmind-referee` | Prebuilt binary (Master + Arena + web UI). Debug build, stripped — the release build hits an unrelated toolchain bug on this machine (a `ring`/rustls C-compiler assembler mismatch); the referee does no heavy compute itself, so this costs nothing at runtime. |
| `data/` | Grid pool, riddle banks, MNIST digit assets, `game_config.json` — read at runtime relative to wherever you run the binary from. **Run the binary from inside `server/`,** or these won't be found. |
| `broker/` | The p2p message broker (`server.py`) every board/Master/Arena talks through. |
| `genesis/` | The Genesis simulated-arm server source (a separately-owned codebase, included here so a full server-role machine is self-contained). Optional and cosmetic — only needed on an arena machine that wants the simulated-arm visual; the referee runs identically without it. Has its own setup, see `genesis/QUICKSTART.md`. |
| `example_grid.json`, `example_pools_config.json`, `kv260_test_*.json` | Fixture files for `--config` mode / local testing. |
| `operators-guide.md` | The general reference — every feature, timing/scoring rules, troubleshooting table. |
| `6-board-3-machine-tournament-guide.md` | **Start here for a real event**: exact steps for 6 teams across 3 machines, a 10-grid pool, and a full single-match walkthrough with who does what. |
| `manual-demo-walkthrough.md`, `three-machine-demo-guide.md`, `kv260-real-hardware-demo-guide.md` | Smaller rehearsal/practice-run guides for specific test scenarios. |
| `setup-server.sh` | One-time setup (see Quick start). |

**Not included:** `static/*.html` (the operator console, scoreboard, and
arena web UIs) — these are compiled directly into the `gridmind-referee`
binary (`include_str!` at build time in the Rust source), so there's
nothing to copy separately, and no way to edit them without rebuilding
from source.

## What's in `client/`

| Path | What it is |
|---|---|
| `notebooks/PYNQ_301-*.ipynb` | Vision detection tuning notebook (local practice, no networking) — two variants (standard + LLM-assisted). |
| `notebooks/PYNQ_302-Referee_Match_Client.ipynb` | The real competition client — detection (all four approaches, switchable via a dropdown) + wire protocol + Genesis, in one notebook. Useful for comparing approaches on a single board. |
| `notebooks/PYNQ_302-<approach>-<Red\|Blue>.ipynb` | Same client, but with one detection approach fixed (no dropdown) and `TEAM_NAME` pre-set to `'red'`/`'blue'` — 8 files total (`yolo_full_frame`, `yolo_grid_crops`, `aruco_border`, `aruco_per_card`, x 2 colors). Shows a fixed stage tracker (Join -> Riddle -> Free Hint -> Play/Wait sub-stages) instead of free-form status text, for a simpler widget GUI during a live demo. The color names are unrelated to Genesis's own `team_red`/`team_blue` arm assignment, which the referee decides dynamically per match. |
| `pynqp2p_pkg/` | The `pynqp2p` client library + a vendored `getmac` wheel (no internet needed on the board). |
| `pynqsim_pkg/` | The `pynqsim` Genesis client library + a vendored `requests` wheel — **only needed if this board's arena has Genesis configured.** |
| `student-competition-guide.md` | The full rulebook: rules, scoring, tournament format, wire protocol, rescue code snippets. |
| `setup-client.sh` | One-time setup (see Quick start). |

## What still needs installing (can't be pre-packaged)

- **Broker's Flask dependency** (`server/broker/requirements.txt`) — not
  vendored, because a compiled-wheel match can't be guaranteed offline
  without knowing the target machine's exact Python version. `setup-server.sh`
  creates a venv at `broker/venv` and installs Flask into that — modern
  distros (PEP 668, "externally-managed-environment") refuse a system-wide
  `pip install` outright, and installing system-wide would be the wrong
  call even where it's still allowed. Low risk either way — Flask is an
  extremely standard package — but it stays isolated in its own venv.
- **`requests` on each PYNQ board** — assumed already present (true on
  every PYNQ image checked so far); `pynqp2p`'s own code imports it
  directly. `setup-client.sh` checks and warns if it's missing.
- **DPU overlay / YOLO model files / VOC class list** (`dpu.bit`,
  `voc_classes.txt`, etc.) and the physical ArUco marker sheets
  (`aruco-reference.html`, in `bootcamp_sessions/PYNQ 301 - Memory Game
  Grid Detection/` in the main repo) — these are standard bootcamp
  training-image content already on the boards, not part of this package.
- **Genesis's own Python dependencies** (`genesis-world`, `torch`, and a
  GPU-backend choice — `cpu`/`amdgpu`/`cuda`) — the *source* is included
  (`server/genesis/`), but these aren't vendored, since the right `torch`
  build depends on that specific machine's GPU/ROCm/CUDA setup. See
  `server/genesis/QUICKSTART.md`. Entirely optional and cosmetic either
  way; the referee runs identically without Genesis at all.

## Deliberately NOT included: `mnist_digits_0-9/` on the client

`PYNQ_302`'s paid-hint debug panel has a "Saved Image" mode for testing a
team's own digit-decoding pipeline against known images without triggering
a real hint request. The sample set this pairs with in earlier drafts of
this package (`board_upload/mnist_digits_0-9/` in the main repo) is
**byte-identical to `server/data/mnist_digits/`** — the exact images the
referee actually renders hints from, not representative practice photos.
Shipping that to every team would hand them the literal answer key: exact
pixel-matching against 10 known images beats building a real classifier,
defeating the point of the hint being a vision challenge. Left out on
purpose. Teams can still exercise "Saved Image" mode by supplying their
own sample digit photos (any 0-9 set in the same style/size works) --
or just test the decoder against real `hint_response` images during a
practice match instead.

## A note on offline vs. online

The **client** side (PYNQ boards) is fully vendored for offline install,
since boards are the ones most likely stuck on a lab-only network with no
internet. The **server** side assumes whoever's setting up the
broker/Master/Arena machine(s) has at least occasional internet access for
the one `broker/venv` Flask install — if that's wrong for your setup, let me
know and I'll vendor those wheels too (need to know the exact Python
version on that machine first, since Flask's dependency chain includes
compiled extensions).
