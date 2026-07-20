# GridMind — Student Implementation Guide: What's Given vs. What You Build

This is the "who does what" map for teams building a GridMind player. If you're
looking for exact message shapes, see `student-api-reference.html` — that's the
full wire protocol reference. This doc is about scope: what the referee system
already does for you, and what's actually your team's engineering work.

## The referee validates your claim. It never runs its own vision model.

This is the single most important thing to understand about the whole game.
When you flip two cards, **you** run detection on both, **you** decide if
they match, and you send that decision as a single claim. The referee checks
your claim against its own golden answer key and tells you if you were right.
It is not silently double-checking your camera feed — if your vision model
misdetects something, that mistake is entirely yours to make (and to learn from
via the `no_match` response, which tells you the true classes so you can
self-correct your own board memory).

## What the referee/system gives you

| Provided | What it does | Where |
|---|---|---|
| `pynqp2p.register/send/receive_all` | Networking — you never open a raw socket | Python package on the board |
| Arena Agent | Validates your `flip`/`flip_both` requests, physically-coordinates the human referee flipping the real card, broadcasts `card_revealed` to both teams, validates your `report_result` claim against the golden grid, tracks turn order/timeouts, generates hint riddles | `gridmind-referee` (Rust, runs off-board) |
| Scoring engine | Streak bonus, wrong-match penalty, response-time tier, hint cost/cap enforcement — you never compute your own score | same |
| Turn/timeout enforcement | 120-second turn window; if you do nothing, the referee ends your turn for you (and now scores that decision by how much of the window you used) | same |
| Tournament orchestration | Pools, round-robin, Grand Final, advancing winners — you just keep responding to `game_start`/`game_over` on the same connection | Master process |
| `join_competition` (optional) | Self-reports your MAC to the operator so they don't have to type it in by hand | your notebook, `RefereeClient.join_competition` |
| A working reference client | The full notebook (`PYNQ_302-Referee_Match_Client.ipynb`) is a genuinely working GridMind player, not a stub — if your team is starting from scratch, it's fair game to read as a reference for wire-protocol usage. Whether you build on it directly or treat it as a spec to reimplement is your team's call. | `PYNQ_302-Referee_Match_Client.ipynb` |

## What your team has to build

| Your job | Why it's not provided | Hardest part |
|---|---|---|
| **Vision detection** — turning a camera frame (or a crop of one) into an object class name | The referee has no camera and no vision model. It only knows the grid's answer key; it never sees what your camera sees. | Getting a clean crop of just one grid cell, and picking a confidence threshold that doesn't waste your turn on a "no object detected" read |
| **Board memory** — remembering every `card_revealed` you've ever seen, including the opponent's flips | The referee broadcasts revealed positions to both teams equally; it doesn't maintain your memory for you | Every `card_revealed` — yours or theirs — needs its own detection call, or you lose the "shared visibility" advantage the game rules give you |
| **Comparison logic** — deciding `match` vs `no_match` from your own two detections | The referee only validates your claim against ground truth after the fact; it never compares for you | Handling low-confidence detections gracefully instead of guessing blind |
| **Position-choice strategy** — which two cells to flip each turn | This is the actual game — the referee has no opinion on strategy | Balancing "flip a guaranteed pair from memory" vs. "explore an unknown cell" as the board fills in |
| **Turn loop / state machine** — waiting for `your_turn`, handling `wait`, driving the flip→detect→report cycle, reacting to `match` (turn continues) vs `no_match` (turn ends) | The referee tells you whose turn it is; it doesn't run your loop for you | Concurrency: you're polling for messages *and* possibly mid-detection *and* possibly mid-turn-timeout, all at once |
| **Pre-game riddle solving** — turning a `pregame_riddle` message into a single-word answer fast enough to call out first | The referee generates and delivers the riddle text; it does not hand you the answer or judge it automatically — a human operator does that manually based on who calls out the answer first | Genuinely hard riddles, not simple ones — this is meant to be a real LLM call, and speed matters since it's a race |
| **Free-hint QR assembly** — decoding each `free_hint_fragment`'s QR image (`decode_qr_png_base64` in the reference notebook) and concatenating them in the right order into a riddle describing an object and its rough grid quadrant | The referee delivers the raw QR-encoded fragments (grid-derived, so it's always about a real object); it does not assemble or interpret them for you | Handling fragments arriving out of order, and interpreting the assembled riddle once you have it |
| **Paid-hint digit reading** — turning `hint_response`'s two base64 PNG digit images into a row/column guess | The referee renders the images; it does not hand you the row/column as data | Small hand-drawn-style digits, human-readable directly, but a proper solution reads them programmatically rather than eyeballing every time |
| **MAC self-discovery** (only if using `join_competition`) | `pynqp2p.get_id()` gives you your own MAC; wiring it into `join_competition(secret)` at connect time is a couple of lines, but it's still your notebook's code, not the referee's | Optional — skip it and the operator just types your MAC in manually, no penalty |

## Object classes you'll actually see

The referee's grid is config-driven — whatever the event's grid JSON says — but
it's always drawn from the VOC-20 class list your vision model was trained on:
`aeroplane, bicycle, bird, boat, bottle, bus, car, cat, chair, cow,
diningtable, dog, horse, motorbike, person, pottedplant, sheep, sofa, train,
tvmonitor`. Ask the RC team which subset your event's physical grid actually
uses — don't assume all 20 are in play.

## Two ways to flip — pick one per turn, not mixed mid-turn

`flip{team,pos}` (one at a time, two round-trips) and `flip_both{team,pos1,pos2}`
(both at once, atomic validation, only valid as your turn's *first* action) are
both fully supported. `flip_both` exists to cut the round-trip time between your
two flips — worth using if your team's detection loop is a bottleneck, since the
referee no longer waits for your first card's detection before physically
flipping your second card.

## If you get stuck

Ask an RC team member during office hours. If a specific piece (vision, wire
protocol, the turn loop, strategy, or hints) is the actual blocker, there are
drop-in reference modules for each in `student-rescue-modules.md` — using one
doesn't mean giving up on the rest of the challenge.
