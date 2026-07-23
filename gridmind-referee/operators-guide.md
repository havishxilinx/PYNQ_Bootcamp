# GridMind — Operator's Guide (Competition Day)

This is the guide for whoever is running the tournament from the operator console —
not a student, not a spectator. It covers the real event-day path (live web
registration), not the `--config` test path used in `kv260-real-hardware-demo-guide.md`
and `three-machine-demo-guide.md` (those are for practice runs with a fixed, known
team list — skip straight to those if that's what you're doing instead).

## Before you start: build your grid pool

**Every scheduled match gets its own grid, randomly assigned from a pool the moment registration closes** — drop as many grid files as you want under `gridmind-referee/data/grids/*.json` ahead of time. Each is plain JSON:
```json
{
  "positions": {
    "A1": "dog", "A2": "bottle", "A3": "person", ...
  }
}
```
Build each with as many positions as your physical grid has cards, using object
classes your teams' boards can actually detect (VOC classes — see
`student-competition-guide.md`). An odd total leaves one position
permanently unmatched (integer division in `total_pairs()`), so use an even count.

**The pool must not be empty** — `Close Registration` fails loudly if `data/grids/` has no `.json` files in it, rather than silently falling back to anything. Once a match's grid is assigned, it's recorded permanently in the schedule — even if you restart the Master (see below), the same match keeps the same grid.

## Registering Tuesday, competing Thursday

Registration state (teams, pools, secrets, and the schedule once closed) is saved to `gridmind-referee/data/tournament_state.json` after every action — you don't need to keep the Master process running between sessions. **Just start it again** on Thursday with the same command; it detects the save file automatically and picks up exactly where Tuesday left off (straight into the Schedule view if registration was already closed, or back into the Registration screen if it wasn't). To start completely fresh instead (e.g. a rehearsal run), add `--fresh` — this ignores the save file for that run without deleting it.

## Step 1 — Start the broker

The p2p broker (`server.py`, `pynqp2p` library) is distributed separately
from this directory — **TODO(havish): fill in where organizers get it for
event day.** Once you have it:

```bash
cd <path-to-broker>
venv/bin/python server.py --host 0.0.0.0 --port 35050 --key <event-key>
```
Pick a real key for the event (not `demokey`, which is fine for practice runs
only). Every arena, the Master, and every student board must be given this same
key. **Watch:** this terminal is the lowest-level view of the whole system —
every `/send`/`/receive_all` call from anyone shows up here. If a team claims
"nothing is happening," this is where you check whether a message ever arrived.

## Step 2 — Start the Master

```bash
cd gridmind-referee
./target/debug/gridmind-referee master --server 127.0.0.1:35050 --key <event-key> \
  --id master-referee --web-port 38800
```
No `--config` flag here — omitting it is what puts the Master into the live
registration flow instead of loading a static team list.

Open:
- **Operator console**: `http://<this-machine-ip>:38800/operator`
- **Public scoreboard**: `http://<this-machine-ip>:38800/` (put this on the
  projector — dark theme, AMD-red accents, meant to be read from across a room)

## Step 3 — Start both arenas

```bash
./target/debug/gridmind-referee arena --server 127.0.0.1:35050 --key <event-key> \
  --id arena-1-referee --master-id master-referee --arena-num 1
./target/debug/gridmind-referee arena --server 127.0.0.1:35050 --key <event-key> \
  --id arena-2-referee --master-id master-referee --arena-num 2
```
If this arena has a Genesis simulated-arm server co-located with it, add
`--genesis-url http://127.0.0.1:9005` (its address) and, if that Genesis
server's `GENESIS_ADMIN_PASSWORD` differs from the default, also
`--genesis-admin-password <password>`. Each Genesis server can only run
one active competition at a time — never point two arenas at the same
Genesis server. Genesis is purely cosmetic: teams' boards each join its
competition scene and get real per-flip arm animation, but a Genesis
outage or misconfiguration never affects the real match or score.

If that Genesis server also runs its separate live-viewer/stream process
on a non-default port, add `--genesis-stream-port <port>` (defaults to
Genesis's own default, `8080`) — the arena UI (`/arena?arena=N`) then
embeds a live video feed of the match automatically. This requires a
Genesis server running the competition-mode streaming fix (registers its
simulation under a fixed `"competition"` key); against an unpatched
Genesis server the video feed silently doesn't appear, nothing else is
affected.

**The arena UI's layout depends on whether `--genesis-url` was passed at
all** (not on whether the video is actually showing right now): with
Genesis configured, it stays the compact strip you're used to, leaving
room for the video below. Without `--genesis-url` for that arena, there's
no video to reserve space for, so the arena page instead shows the same
fuller view as the master scoreboard — hint icons, the match-elapsed
timer, the live card grid, and both pools' standings/schedule underneath.
This is decided once per match, not per frame, so a Genesis server that's
just slow to start doesn't cause the layout to jump around mid-match.

Each arena is silent until a match is actually assigned to it — that's normal,
not a hang. Arenas run independently (each has its own assignment thread), so
one arena's popup sitting open doesn't freeze the other.

## Step 4 — Registration

The operator console starts in the **Registration** view. As teams check in:

1. Click **Register Team**, enter the team name and student names.
2. New teams auto-balance across Pool 1 / Pool 2. If you need to override
   (e.g. keep two teams from the same school apart), use **Move to Pool** —
   this is safe even if it results in an uneven split; the two-team dry-run
   normalization only kicks in later, at exactly-2-teams-total.
3. Each registered team is issued an **8-character join secret**, shown right
   there in the registration view. **Write these down or have the team write
   theirs down** — they'll paste it into their notebook's `TEAM_SECRET` field.
   This is the only place the secret is surfaced in the live flow (in
   `--config` mode it's printed to the Master's console instead, since that
   path skips this screen).
4. When everyone's checked in, click **Close Registration**. This is a one-way
   door for that run — the tournament schedule (pools, matches) is generated
   from whoever's registered at that moment.

**Special case — exactly 2 teams total:** the tournament engine detects this
automatically and skips pools/round-robin entirely, going straight to a single
Grand-Final-style match. No operator action needed; it just happens.

## Step 5 — Start the tournament

Click **Start Tournament** in the operator console (this is the
`/api/start-tournament` call). The console switches to the **Schedule** view:
each match shows as `Locked` → `Ready` → `Live` → `Complete`.

## Step 6 — Run each match

A match's row can show **Ready** long before either team is actually
present — the schedule is built the moment registration closes, which may
be days before the tournament itself runs (team formation/registration one
day, students building their client the next, the tournament the day
after that). The pregame ceremony reflects this: **nothing timed starts
until both teams are known to have joined.**

- **A "Waiting for both teams to connect" panel** appears above that
  arena's admin panel the instant the match goes Ready, showing a
  checkmark per team as they connect. Nothing else happens yet — no
  riddle, no countdown — until both show a checkmark.
- **MACs are detected automatically** within about a second of each team's
  board clicking Connect in their notebook (assuming they filled in
  `TEAM_SECRET` and `MASTER_ID`) — this is the Join Competition feature; it
  polls `/api/join-status` in the background. If a team's board can't
  self-report (wrong secret, `MASTER_ID` not set, or some other client-side
  issue), use **Mark as Joined** in that same panel — type their name and
  MAC (get it from them out loud or from their board) and submit; this
  records it exactly as if they'd self-reported, and unblocks the ceremony
  the same way.
- **Once both teams are known**, the panel switches to "both teams
  joined -- ready to start" with a **Start Pregame** button. The riddle does
  **not** fire automatically the instant both MACs are known — click Start
  Pregame yourself once you've confirmed both boards are actually ready.
  After you click it, both teams receive a real riddle and the scoreboard
  shows a live puzzle-race countdown with the riddle text — this is real,
  not a placeholder. **Judging is entirely manual**: whichever team calls
  out the correct answer first, tell you out loud — there's no automatic
  check, so listen for it yourself. If nobody gets it, decide however your
  event's rules say to (a coin flip is fine).
- If a team says they never received the riddle (or the free hint, once
  you're past that stage), use the **Resend** button in that same panel —
  or **Restart** for a fresh riddle/hint and a reset timer if something's
  really stuck.
- Click the Ready row to open the match-assign popup once both teams are
  in. MAC fields auto-fill green (or stay manually editable as a fallback).
  Pick the puzzle-race winner, click **Confirm Winner**. This calls
  `/api/start-match` with `{arena, winner, team_a_mac, team_b_mac}` —
  **arena must match which arena's Ready row you clicked**; the popup
  handles this for you, you don't type an arena number. This records the
  winner and MACs but does **not** send the free hint yet.
- The panel now shows "winner confirmed" with a **Start Match** button —
  a deliberately separate action, so confirming the winner doesn't force
  you straight into the free-hint clock. Click it whenever you're actually
  ready to proceed.
- Clicking Start Match sends both teams a shared **free hint** (a real
  object + rough location on that match's actual grid, delivered as
  several plain-text fragments) — a live countdown shows on the
  scoreboard. No further operator action needed for this stage; the
  match starts automatically once the free-hint window finishes.

**Watch, once the match starts:**
- **The scoreboard** — live grid, turn indicator, scores, streak, a turn timer
  counting down from **120 seconds** (not 60 — this was extended mid-project;
  if you're referencing an older Confluence guide it may still say 60s, see
  the note below).
- **The per-arena view** (`http://<this-machine-ip>:38800/arena?arena=1` or
  `?arena=2`) — the same live score/timer strip as the scoreboard, but scoped
  to one arena and full-screen, meant to run on the machine co-located with
  that arena's camera/Genesis setup. It also shows the pre-game puzzle-race
  and free-hint countdowns (with the riddle text itself during the puzzle
  race), and a color-coded response-time tier readout that updates live as
  the turn timer counts down — useful for showing a team in real time which
  scoring tier they're currently in.
- **The "FLIP CARDS AT X AND Y NOW" banner** — appears on both the scoreboard
  and the per-arena view the instant a team's second card is revealed. This is
  the physical referee's cue to flip the actual cards on the board; it clears
  once that pair's result is reported.
- **Each board's own notebook status panel** — "Your turn" / "Waiting" flips
  back and forth automatically once a team has flipped their Play Mode toggle
  on (Wait Mode is the default; a team stuck in Wait Mode just looks
  frozen — check their toggle if a team seems unresponsive).
- **The Master's terminal** — one `[arena N, pool 0] X/7 pairs, scores: {...}`
  line per action. This is your ground truth if the UI ever looks stale.

Scoring is claim-based: each team compares its own two detections and reports
`match`/`no_match`; the referee validates against the grid's golden answer key.
Two independent numbers add together on a correct match; every other outcome
scores a single flat number with no speed component at all:

- **Correct match:** streak count (1st in a row this turn = 1, 2nd = 2, 3rd =
  3, ...) **plus** a speed bonus based on how fast it came back (0–40s: +2,
  41–80s: 0, 81–120s: −2). This is the *only* outcome speed affects.
- **Wrong match claim:** a flat **−2**, always, regardless of speed.
- **Correct decline** ("no_match", genuinely not a pair): **0**, always —
  declining isn't the hard part of this game, so there's nothing to reward
  or punish there.
- **Timeout** (no action at all): a flat **−3**, always.

If a team asks why a fast wrong guess didn't score better than a slow one —
it's deliberate. Speed used to apply to every outcome, which meant a fast
wrong guess could still net a small positive score; now speed only ever
rewards being *right*. The first speed tier is 40 seconds wide (not 20)
because the first 20 seconds of every action are treated as unavoidable
physical-flip/camera overhead and don't count against the team — see
"Tuning timing and scoring" below.

## Tuning timing and scoring before the event

All the timing/scoring constants above — turn timeout, the physical-flip
allowance, puzzle-race and free-hint window lengths, the response-time tiers,
and the paid-hint cap/cost — live in `gridmind-referee/data/game_config.json`
and can be edited without rebuilding. Edit the file, then (re)start the
Master and arena processes — the config loads once at process start, not
mid-tournament, so gameplay rules can't shift mid-match. If the file is
missing or malformed, the referee falls back to its built-in defaults (the
values already shown throughout this guide) rather than crashing.

## Live match admin overrides

Available on the arena's admin panel once a match is actually live (not
during pregame — see the pregame-specific Resend/Restart/Start Pregame/Start
Match controls in Step 6):

- **Pause / Resume** — genuinely freezes the turn clock, with zero scoring
  side effect: resuming after any real-world gap (a 10-minute pause included)
  leaves exactly the same time remaining as when you paused. Incoming
  student messages are dropped while paused (not queued for later), so
  there's nothing to "catch up on" after resuming either. Safe to use
  liberally — this is the right first move for almost any mid-match problem.
- **Set Score** — directly overwrites one team's score to an absolute value.
  For correcting a scoring dispute, not for normal play.
- **Finish Now** — ends the match immediately, crediting whoever's currently
  ahead (same tie-break as a natural finish). The tournament schedule
  advances exactly as if the match had ended normally, and the live
  scoreboard/arena view clears right away.
- **Stop (void)** — halts the match with **no result recorded at all**, not
  even "whoever was ahead." The schedule entry reverts to Ready, so the
  exact same matchup gets handed out again the next time this arena is
  free — click the newly-Ready row and go through registration/pregame for
  it again from scratch, riddle and free hint included. Use this over
  Finish when something went genuinely wrong (wrong grid loaded, wrong
  teams assigned, a technical failure mid-match) rather than a normal
  early end where a score should still count.

### If a board dies, disconnects, or otherwise stops responding mid-match

1. **Pause immediately.** This costs nothing — see above. Don't let the other
   team's clock (or the disconnected team's own clock, next time it's their
   turn) run out from something outside their control.
2. **Check whether it's actually dead, or just the widget GUI.** A
   background thread inside the notebook can flood the cell's raw output
   during active play, which in some Jupyter frontends visually buries the
   widget controls even though the board is still fully connected and
   playing correctly — it looks exactly like "my board died" from the
   student's seat. Have them scroll up, or just watch the scoreboard/arena
   page: if their score/turns are still updating there, the board is fine
   and this is a display issue, not a connectivity one.
3. **If it's a genuine reconnect (kernel restart, notebook crash):** the
   student re-runs Connect and the widget cells. The connection and message
   flow resume correctly (same board ID/MAC), but their board's own memory
   of previously-revealed cards is gone — a fresh kernel starts with an
   empty `board_memory`. They'll only know about cards revealed *after*
   reconnecting; this is a real, uncompensated disadvantage, not something
   the referee can restore for them.
4. **Resume once they're back**, and let the match continue.
5. **If it can't be recovered in a reasonable time**, decide between:
   - **Stop** — if the disconnect happened early enough that a full replay
     is fair and there's time for one.
   - **Finish** — if the match is well underway and time is short; credits
     whoever's currently ahead, treating the outage as that team's problem
     to have solved faster. Use judgment on fairness given how the score
     looked right before the disconnect.

## Step 7 — Paid hints (if a team asks about them)

A team can request a paid hint for an object name during their own active
turn. Cost: 1 point per attempt (accepted or rejected), capped at 2 per
match, and only available if their current score is above 0 — a request
outside those conditions is silently ignored (no cost, no response). The
response is two small digit images (row, then column) — not text — so don't
expect to see readable content in the Master's log, just confirmation that
`hint_request`/`hint_response` fired.

## Step 8 — Game over / Grand Final / Champion

Each match ends with `game_over` once all pairs are found; scoreboard and both
boards' panels show the winner and final scores. Pool winners automatically
advance into the Grand Final. Once a Champion is declared, the operator console
and scoreboard both switch to a Champion screen.

**Known quirk: the Master process does not exit after a Champion is
declared** — it deliberately loops forever so the projected trophy screen
stays up on the projector. To run a second tournament that same day, Ctrl+C
the Master and restart it (registration begins fresh).

## Practice Mode — let a team validate their client without a second team

Once you're past registration (any time the console shows the Schedule
view), a **Practice Mode** panel appears above the arena admin panels. A
team can walk up, connect their board as usual, and play a real match
against the referee's own built-in bot ("Referee Bot") instead of another
team — genuinely exercises their client's wire protocol, vision pipeline,
and turn loop against the real referee, with none of it touching pool
standings or the Grand Final.

1. Pick which idle arena to use, enter the team's name and MAC (same as a
   normal match-assign — type it manually or have them `join_competition`
   first), and a grid file (any `data/grids/*.json`).
2. Click **Start Practice Match**. The bot always lets the real team go
   first. When it's the bot's turn, it announces its flip immediately (same
   "FLIP CARDS AT X AND Y NOW" banner a real flip uses) and waits about 6
   seconds — matching the real physical-flip pacing — before reporting its
   (always truthful) result.
3. The match plays out and scores exactly like a real one, visible on the
   scoreboard and that arena's admin panel (pause/stop/finish all work
   normally). Once it ends, the arena resets to idle — nothing about it
   reaches the pool schedule or standings.

**Blocked (409) if:** the tournament hasn't reached the live Schedule phase
yet (Registration/Idle — pre-event validation should use a separate
rehearsal instance instead, see `manual-demo-walkthrough.md`), or the
chosen arena already has a real match or pre-game ceremony running. Pick
the other arena, or wait.

## Troubleshooting quick reference

| Symptom | Likely cause / where to look |
|---|---|
| Team's MAC field won't auto-fill | Wrong `TEAM_SECRET` pasted, board hasn't clicked Connect yet, or `MASTER_ID` left blank in their notebook. Enter the MAC manually as a fallback — it doesn't block the match. |
| Board looks connected but never plays its turn | Play Mode toggle is still on Wait Mode — ask them to flip it. |
| Nothing shows up anywhere for an action a team says they took | Check the broker's own terminal (Step 1) — if the message never appears there, it never left their board's network stack (network issue, not a GridMind bug). |
| An arena's popup has been open a long time and the *other* arena also looks stuck | Should not happen post-concurrency-refactor — each arena has its own thread. If it does happen, that's a regression worth flagging, not expected behavior. |
| A score changed on what looked like "nothing happening" (a timeout or decline) | Expected — response-time tier scoring now applies even to timeouts/declines, see Step 6. |
| Need to reference an older planning doc or Confluence page | Some describe an earlier elimination-bracket tournament format or a 60s/no-penalty-timeout rule that no longer applies. Trust this guide and the running code over any older document until that one's updated. |

## Cleanup at end of day

```bash
pkill -f "gridmind-referee (master|arena)"
pkill -f "server.py --host 0.0.0.0 --port 35050"
```
