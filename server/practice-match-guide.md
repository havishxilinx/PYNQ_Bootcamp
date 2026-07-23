# Practice Match Guide — For Student Teams

A Practice Match lets your team play a full, real match against the
referee's own built-in bot opponent ("Referee Bot") — no second team
needed. It exercises the exact same wire protocol, scoring, and turn logic
as a real tournament match. **Nothing about the result counts toward pool
standings** — it exists purely so you can confirm your notebook actually
works against the real referee before you're on the clock in a real match.

## Before you start: when Practice Mode is available

Practice Mode is **only available once the tournament is actually
running** — after the operator has closed registration and clicked Start
Tournament. It doesn't exist during the registration/setup window, and it
can't run on an arena that currently has a real match or pre-game ceremony
in progress. If the operator can't start your practice match yet, that's
why — ask again once pool play is underway, or once your assigned arena
frees up between real matches.

You do **not** need to be a registered tournament team to run a practice
match. Any team name and MAC works — practice matches skip the normal
join/registration check entirely.

## Step 1 — Tell the operator three things

Walk up to the operator console and give them:

1. **Which arena** you want to use (whichever one is currently idle —
   the operator can tell you).
2. **A team name** — anything you like, it doesn't have to match a
   registered team.
3. **Your board's MAC address** — exactly the same value you'll put in
   your notebook's `BOARD_ID_OVERRIDE` (or your board's real MAC if you
   leave that blank). This has to match *exactly*, or the referee's
   messages will never reach you — see Troubleshooting below.

The operator also picks a grid file for your practice match (any file
under `data/grids/`, or any other real grid — it doesn't need to match
what's used in real pool play).

## Step 2 — Configure and connect your notebook

In your notebook's Match Configuration cell:

```python
SERVER = '<the event's broker address>'   # same as a real match
BROKER_KEY = '<the event's broker key>'   # same as a real match
REFEREE_ID = 'arena-<N>-referee'          # N = whichever arena the operator started your practice match on
MASTER_ID = 'master-referee'              # unchanged
TEAM_NAME = '<the team name you gave the operator>'
TEAM_SECRET = ''                          # leave blank -- practice matches don't check it
BOARD_ID_OVERRIDE = '<the MAC you gave the operator>'   # or leave blank to use your board's real MAC
```

Click **Connect**, then **Start**, then switch to **Play Mode** — exactly
like a real match. It doesn't matter whether you connect before or after
the operator starts the practice match on their end; the referee's first
message to you (`game_start`) just waits for you to poll if you're not
listening yet. But connecting first means you won't burn any of your
120-second turn clock before you're even watching for messages.

## What's different from a real match

- **No pre-game riddle, no free hint, no puzzle race.** The match starts
  immediately once the operator clicks Start Practice Match.
- **No Genesis simulated arm**, even if Genesis is configured for that
  arena — there's no second robot for a bot opponent to belong to.
- **Your opponent is "Referee Bot"** — it reads the grid's real answer key
  directly (it has no camera or vision model to fool), so it never
  misdetects a card. It does pace itself with a ~6 second "thinking" delay
  per turn so it doesn't feel instant, and it plays the same known-pair
  strategy this guide recommends as a baseline (Section 13, Module D) —
  it is a genuinely tough opponent, not an easy one. The point of a
  practice match is to prove your client survives real turns, hints, and
  edge cases correctly — not necessarily to win.
- **Nothing is recorded.** No pool standings, no schedule entry, no
  Grand Final eligibility. Run as many practice matches as you want.

## Troubleshooting

**Nothing happens after Connect/Start — no `game_start` ever arrives.**
Almost always a MAC mismatch: the `BOARD_ID_OVERRIDE` (or your board's
real MAC, if you left that blank) doesn't exactly match what the operator
typed into the practice match form. Messages are routed by that exact
string — even a case or whitespace difference means your board is
listening under a different ID than the referee is sending to. Confirm
the exact MAC with the operator and re-check your config cell.

**Wrong arena / still nothing.** Double check `REFEREE_ID` matches the
arena number the operator actually started your practice match on —
`arena-1-referee` and `arena-2-referee` are two completely independent
listeners.

**Operator says the arena is busy.** Practice Mode is blocked on an arena
that already has a live match or pre-game ceremony running. Ask for the
other arena, or wait for the current match to finish.
