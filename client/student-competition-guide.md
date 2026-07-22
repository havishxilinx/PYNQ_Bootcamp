# GridMind — Official Rulebook & Player's Guide

Everything a team needs, from the first line of code to the final match: what
the game is, exactly how it's scored, what the referee does for you vs. what
you build, the full wire protocol, and drop-in rescue code if you get stuck.
One document, verified directly against the actual referee software
(`gridmind-referee`) that runs the real event — not against any older
planning doc, slide, or Confluence page.

> If something you read elsewhere contradicts this document, **this document
> and the RC Team win.** Flag a real discrepancy in this doc itself to the RC
> Team rather than guessing.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Components](#2-components)
3. [Objective](#3-objective)
4. [Setup — The Pre-Game Window](#4-setup--the-pre-game-window)
5. [How to Play a Turn](#5-how-to-play-a-turn)
6. [Scoring](#6-scoring)
7. [Hints](#7-hints)
8. [Tournament Format](#8-tournament-format)
9. [Special Rules & Edge Cases](#9-special-rules--edge-cases)
10. [Optional: Genesis Simulated Robot Arm](#10-optional-genesis-simulated-robot-arm)
11. [What's Given vs. What You Build](#11-whats-given-vs-what-you-build)
12. [Wire Protocol Reference](#12-wire-protocol-reference)
13. [Rescue Modules](#13-rescue-modules)
14. [Glossary](#14-glossary)
15. [FAQ](#15-faq)

---

## 1. Overview

GridMind is a two-team AI card-matching competition. A grid of face-down
cards sits between two teams; each object appears on exactly two cards
(a classic memory-match game). Each team's board runs its own vision
model — there is no shared "referee vision system" watching the grid. When a
card is revealed, **both teams** get to look at it (with their own camera and
their own model) and remember what they saw. On your turn, you pick two
cards, your board tells the referee whether they match, and the referee
checks your answer against the real, hidden answer key.

**The referee never runs its own vision model and never tells you what's on
a card.** It only validates the claim you send it. This is the single most
important rule in the whole game — see [Section 11](#11-whats-given-vs-what-you-build)
for the full breakdown of what the referee does for you vs. what your team
has to build.

## 2. Components

- **The grid** — a physical board of face-down cards, e.g. a 5×6 layout
  (exact size and object classes are set per event by the RC Team). Positions
  are labeled `<row letter><col number>` (e.g. `B3`, `E6`) — a letter for the
  row, a number for the column.
- **Object classes** — whatever the event's grid file says, but always drawn
  from the VOC-20 class list your vision model was trained on: `aeroplane,
  bicycle, bird, boat, bottle, bus, car, cat, chair, cow, diningtable, dog,
  horse, motorbike, person, pottedplant, sheep, sofa, train, tvmonitor`. Ask
  the RC Team which subset your event's physical grid actually uses — don't
  assume all 20 are in play.
- **Two teams**, each with one board (the "player") and, if used, a real
  physical referee at the table to flip the actual cards.
- **The Arena Agent** ("the referee") — the `gridmind-referee` software that
  validates every action, tracks the score, and enforces the clock. It is
  not a physical object, but it's the authority the whole match runs against.
- **(Optional)** a Genesis simulated robot arm per arena, purely for visual
  flavor — see [Section 10](#10-optional-genesis-simulated-robot-arm).

## 3. Objective

Score more points than your opponent by the time every pair on the grid has
been found. Points come from finding pairs quickly and correctly (see
[Scoring](#6-scoring)) — it is not simply "whoever finds the most pairs,"
since streaks, response speed, wrong guesses, and hints all move the score
independently of pair count.

## 4. Setup — The Pre-Game Window

Nothing in this window starts until **both teams for that match have
connected** — the referee sends nothing timed while it's still just
waiting for one or both boards to show up (your match's schedule slot can
exist long before you're actually at the table). Once both are in, the
human operator explicitly starts the riddle race — it does **not** fire
the instant your boards connect, so don't worry if there's a short pause
after you both show as joined. Once started, two things happen here, in
order, before the match clock starts:

1. **The riddle race** (decides who goes first). The referee sends both
   teams the identical hard riddle (`pregame_riddle`) with a single-word
   answer. You get **120 seconds** to solve it with your own LLM.
   **Judging is entirely manual** — there is no wire message for submitting
   an answer. Whichever team calls out the correct answer first, out loud,
   to the human operator, wins and goes first. If nobody gets it, the
   operator decides (a coin flip, typically).
2. **The free hint** (shared, not competitive). Right after, both teams
   receive an identical hint — a riddle describing one real object on the
   grid and its rough quadrant, delivered as several plain-text fragments
   (`free_hint_fragment`) you assemble yourself in order. You get **60
   seconds** for this stage. It doesn't affect turn order; it's free intel
   for both teams equally.

The match itself begins with a `game_start` message naming both teams in
turn order (whoever won the riddle race goes first).

## 5. How to Play a Turn

1. **Pick two grid positions** and tell the referee — either `flip` twice
   (one position, two round-trips) or `flip_both` for both positions in one
   message (only valid as the very first action of a turn — see
   [Section 12](#12-wire-protocol-reference) for exact shapes).
2. **A human referee physically flips the real card(s).** You'll get a
   `card_revealed` broadcast for each position — **with no class label.**
   Run your own vision model on your own camera frame to figure out what's
   there.
3. **Compare your two detections yourself.** Report your claim: `match` or
   `no_match`.
4. **The referee checks your claim against the real answer key:**
   - **Correct match:** points awarded, and **your turn continues** — go
     back to step 1.
   - **Wrong claim, or genuinely no match:** your turn ends. The `no_match`
     response still includes the two cards' true classes on the wire, but
     **don't build your strategy around it** — the reference client
     deliberately does not use it to auto-correct your board memory
     anymore, and the referee is expected to stop sending it as golden
     truth in a future update. A misdetected pair needs to be caught by
     your own model re-observing that position, not by reading the
     answer off a wrong guess.
5. **You keep receiving every `card_revealed` broadcast even during the
   opponent's turn.** Both teams see everything that's ever flipped — this
   is deliberate. A team that ignores the opponent's turn is throwing away
   free information.
6. **You have 120 seconds** from the moment you're handed the turn to act.
   If you go over, the referee ends your turn for you (see
   [Scoring](#6-scoring) — this is now actively penalized, not neutral).

## 6. Scoring

### Correct matches — streak bonus

Each match in a row within the same turn is worth one more point than the
last:

| Matches in a row this turn | Points for that turn |
|---|---|
| 1 | 1 |
| 2 | 3 (1+2) |
| 3 | 6 (1+2+3) |
| 4 | 10 (1+2+3+4) |

### Wrong match

**−1 point**, and your turn ends immediately.

### Response-time tier — applies to every turn outcome, including timeouts

On top of the above, how fast you acted after being handed the turn adds or
subtracts points — **this applies even if you time out or decline**, which
is a real behavior worth knowing about:

| Time elapsed when you acted (or ran out) | Bonus/penalty |
|---|---|
| 0–40s | +2 |
| 41–60s | +1 |
| 61–80s | 0 |
| 81–100s | −1 |
| 101–120s | −2 |
| Beyond 120s / full timeout | −3 |

The first tier is 40 seconds wide, not 20 — the first 20 seconds of every
action are treated as unavoidable physical-flip/camera overhead and don't
count against you. Everything after that is real decision + detection time.
This stacks with the streak bonus on a correct match, adds to the −1 flat
penalty on a wrong match, and — a real behavior worth knowing — also applies
to declines and full timeouts, which some older docs describe as always
scoring 0 regardless of speed. That is no longer true: a team that always
sits and waits out the full clock is actively penalized every turn, not just
missing out on points.

**These exact numbers (turn timeout, the 20s allowance, and every tier
boundary/bonus above) are tunable per event via a config file** and could be
adjusted before your specific event — the numbers above are what ships by
default. If your event's numbers differ, the RC Team will tell you; the
mechanism (a 6-tier scale based on how fast you act) stays the same either
way.

## 7. Hints

### Paid hints (mid-match, on-demand)

Request a hint for a specific object name. It costs **1 point**, deducted
immediately, capped at **2 attempts per match**, and only available if your
score is currently above 0. You get back **two small digit images**
(base64-encoded PNGs) — one for the row number, one for the column number —
not text. Read them yourself (row letters map to numbers the same way as
everywhere else: A=1, B=2...). If your request is outside those conditions
(score too low, already used both attempts, unknown/mistyped object name —
matching is case-sensitive), it's either silently ignored (no cost) or
rejected with a reason (still costs the point — see
[Section 12](#12-wire-protocol-reference) for the exact rejection reasons).

**You can request a hint two ways, and both are fully supported:**

1. **During your own active turn** — resolves immediately, but the
   request/response round trip comes out of your 120-second turn clock, same
   as everything else you do on your turn.
2. **While it's the opponent's turn (recommended)** — send the exact same
   `hint_request` message any time you're *not* active. The referee queues
   it silently (no response yet) and automatically resolves it the instant
   your turn actually starts — the `hint_response`/`hint_rejected` message
   arrives **before** your `your_turn` message, so it costs you nothing off
   your own clock. Only your latest queued request is kept if you send more
   than one while waiting. All the same conditions (score > 0, under the
   cap, valid object) are checked at the moment it resolves, not when you
   queued it — so if the object gets fully revealed by the other team's
   flips before your turn starts, you may still get "object already fully
   resolved" (still costs the point).

There's no way to know your queued hint was accepted until it resolves —
don't block waiting for a response you sent while you weren't active.

### Free hints (pre-game, shared)

See [Section 4](#4-setup--the-pre-game-window) above — one shared hint per
match, delivered as plain-text fragments you assemble in order, no cost,
not competitive.

## 8. Tournament Format

- Teams are split into **two pools**. Within a pool, everyone plays everyone
  else once (round robin).
- The team with the best record in each pool advances to a single
  **Grand Final** match.
- **Exception:** if only two teams total are registered, pools are skipped
  entirely and it goes straight to one Grand-Final-style match.
- There is no elimination round — a bad match doesn't knock you out of your
  pool.
- **Tiebreakers within a pool:** most wins, then most total pairs matched.

## 9. Special Rules & Edge Cases

1. **No human input once the match clock starts.** Your board operates on
   its own from `game_start` to `game_over`.
2. **A correct match keeps your turn.** Keep flipping until you get a wrong
   claim or the match ends.
3. **A wrong claim costs 1 point and ends your turn immediately.**
4. **Both teams receive every card reveal, regardless of whose turn it
   was.** Use that information — it's free.
5. **Two flip protocols, side by side, not a replacement for each other.**
   `flip{team,pos}` (one position, two round-trips) and
   `flip_both{team,pos1,pos2}` (both positions in one message, first action
   of a turn only) are both fully supported and tested. `flip_both` exists
   to cut round-trip time — worth using if your detection loop is a
   bottleneck, since the referee no longer waits for your first card's
   detection before physically flipping your second card. You can't mix
   protocols mid-turn (e.g. one single `flip` followed by a `flip_both`).
6. **Paid hints cost 1 point per attempt, capped at 2 per match, and require
   a positive score.** Free hints (pre-game, shared) cost nothing.
7. **The referee's decision is final.** It validates your claim against the
   real answer key; it does not take your word for it, and it does not run
   its own vision model.
8. **Genesis (if used) never affects scoring.** See
   [Section 10](#10-optional-genesis-simulated-robot-arm).

## 10. Optional: Genesis Simulated Robot Arm

**Purely cosmetic** and completely separate from everything above. It never
talks to the referee, never affects your score, and is entirely optional —
skip this section if you just want to compete.

If Genesis is configured for your arena, your `game_start` message includes
a `genesis_team_id` (`"team_red"` or `"team_blue"`) and a `genesis_url`.
Connect directly to the Genesis simulation server — this is a **separate
HTTP connection**, not routed through `pynqp2p` or the referee at all:

```python
from pynqsim import SimulationClient

sim = SimulationClient(genesis_server_ip, port=9002)  # genesis_url from game_start
sim.join_competition(team_id=genesis_team_id)          # "team_red" or "team_blue" from game_start
```

Once joined, `sim.flip_card(row, col)` animates your team's arm actually
flipping that card in the simulated scene, and `sim.end_turn()` hands the
simulated turn to the other team — both purely visual. The referee's
decision about whether cards match is based **only** on the `report_result`
claim you send it, exactly as described in [Section 5](#5-how-to-play-a-turn)
— Genesis has no say in it, and a Genesis call failing or being skipped
never affects your match. Genesis also runs its own internal turn/match
tracking for its animation's sake; ignore it, it's not authoritative.

## 11. What's Given vs. What You Build

This is the "who does what" map for teams building a GridMind player.

### What the referee/system gives you

| Provided | What it does |
|---|---|
| `pynqp2p.register/send/receive_all` | Networking — you never open a raw socket |
| Arena Agent | Validates your `flip`/`flip_both` requests, physically-coordinates the human referee flipping the real card, broadcasts `card_revealed` to both teams, validates your `report_result` claim against the golden grid, tracks turn order/timeouts, generates hint riddles |
| Scoring engine | Streak bonus, wrong-match penalty, response-time tier, hint cost/cap enforcement — you never compute your own score |
| Turn/timeout enforcement | 120-second turn window; if you do nothing, the referee ends your turn for you (and scores that decision by how much of the window you used) |
| Tournament orchestration | Pools, round-robin, Grand Final, advancing winners — you just keep responding to `game_start`/`game_over` on the same connection |
| `join_competition` (optional) | Self-reports your MAC to the operator so they don't have to type it in by hand |
| A working reference client | `PYNQ_302-Referee_Match_Client.ipynb` is a genuinely working GridMind player, not a stub — fair game to read as a reference for wire-protocol usage, whether your team builds on it directly or reimplements from spec. Ten fixed-approach, fixed-color variants also exist (`PYNQ_302-<approach>-<Red\|Blue>.ipynb`, one per detection approach x team color — five approaches: `yolo_full_frame`, `yolo_grid_crops`, `aruco_border`, `aruco_per_card`, `aruco_border_grid_crops`) — same logic, no approach-switching dropdown, and a fixed stage tracker (Join → Riddle → Free Hint → Play/Wait sub-stages) instead of free-form status text, for a simpler widget GUI during a live demo |

### What your team has to build

| Your job | Why it's not provided | Hardest part |
|---|---|---|
| **Vision detection** — turning a camera frame (or a crop of one) into an object class name | The referee has no camera and no vision model. It only knows the grid's answer key; it never sees what your camera sees. | Getting a clean crop of just one grid cell, and picking a confidence threshold that doesn't waste your turn on a "no object detected" read |
| **Board memory** — remembering every `card_revealed` you've ever seen, including the opponent's flips | The referee broadcasts revealed positions to both teams equally; it doesn't maintain your memory for you | Every `card_revealed` — yours or theirs — needs its own detection call, or you lose the "shared visibility" advantage the game rules give you |
| **Comparison logic** — deciding `match` vs `no_match` from your own two detections | The referee only validates your claim against ground truth after the fact; it never compares for you | Handling low-confidence detections gracefully instead of guessing blind |
| **Position-choice strategy** — which two cells to flip each turn | This is the actual game — the referee has no opinion on strategy | Balancing "flip a guaranteed pair from memory" vs. "explore an unknown cell" as the board fills in |
| **Turn loop / state machine** | The referee tells you whose turn it is; it doesn't run your loop for you | Concurrency: you're polling for messages *and* possibly mid-detection *and* possibly mid-turn-timeout, all at once |
| **Pre-game riddle solving** — a single-word answer fast enough to call out first | The referee generates and delivers the riddle text; it does not hand you the answer or judge it automatically | Genuinely hard riddles, not simple ones — this is meant to be a real LLM call, and speed matters since it's a race |
| **Free-hint assembly** — concatenating each `free_hint_fragment`'s text in order | The referee delivers unordered fragments (by `index`); it does not assemble them for you | Handling fragments arriving out of order |
| **Paid-hint digit reading** — turning two base64 PNG digit images into a row/column guess | The referee renders the images; it does not hand you the row/column as data | A proper solution reads them programmatically rather than eyeballing every time |
| **MAC self-discovery** (only if using `join_competition`) | `pynqp2p.get_id()` gives you your own MAC; wiring it in is a couple of lines, but it's still your notebook's code | Optional — skip it and the operator just types your MAC in manually, no penalty |

## 12. Wire Protocol Reference

### Connecting

```python
import pynqp2p

pynqp2p.register("192.168.1.100:5000", "bootcamp2024")   # server IP:port + shared key, from the RC team
my_id = pynqp2p.get_id()                                   # your board's MAC address
print(f"My board ID: {my_id}")   # share this with the RC team so they can register you
```

Every message you send goes to **your arena's referee board ID** (given to
you by the RC Team when your match is set up) via
`pynqp2p.send(referee_id, json_string)`. You read your own inbox with
`pynqp2p.receive_all()`, which returns a list of raw JSON strings (drains
your queue — each message is delivered once). You never talk to the other
team's board or to the Master directly — everything routes through your
Arena Agent.

### Messages you SEND to the referee

| Type | Fields | When |
|---|---|---|
| `flip` | `team, pos` | Choosing a card to flip, on your turn |
| `flip_both` | `team, pos1, pos2` | Alternative to two sequential `flip` calls — both positions at once, atomic validation. Only valid as the first action of a turn. |
| `report_result` | `team, pos1, pos2, cls1, cls2, claim` | After detecting both of your flipped cards, submitting your own match/no_match comparison |
| `hint_request` | `team, object` | Optional. During your own turn, resolves immediately. While waiting (not your turn), it's queued and auto-resolves right before your next `your_turn` — see [Section 7](#7-hints). |
| `join` | `team, mac, secret` | Optional, once, right after connecting (`join_competition`) |

### Messages you RECEIVE from the referee

| Type | Fields | Meaning |
|---|---|---|
| `pregame_riddle` | `riddle` | Pre-game, both teams identically — decides who goes first (judged manually, no answer message exists) |
| `free_hint_fragment` | `index, total, text` | Pre-game, both teams identically — assemble every fragment yourself (plain text, no decoding needed) |
| `game_start` | `teams, total_pairs, robot_id, genesis_team_id?, genesis_url?` | Match beginning; `teams` lists both names in turn order. The three `genesis_*`-adjacent fields are only meaningful when Genesis is configured for this arena — see Section 10. |
| `your_turn` | `flip_num` | It's your turn |
| `wait` | `active_team` | Not your turn — but still watch for `card_revealed` |
| `card_revealed` | `pos` | Broadcast to both teams on every physical flip — no class label, run your own model |
| `invalid` | `reason` | Your last `flip`/`flip_both` request was rejected — pick a different position |
| `match` | `cls, pos1, pos2, scorer, scores, remaining` | A claimed match was confirmed correct |
| `no_match` | `pos1, pos2, cls1, cls2, scores` | Turn ended — wrong claim (penalty) or genuinely no match (no penalty). `cls1`/`cls2` are still golden truth on the wire for now, but treat this as a legacy field — the reference client ignores it, and a future update is expected to stop sending it as an answer key. |
| `hint_response` | `row_digit_png_base64, col_digit_png_base64` | Paid hint accepted — two digit images, not text |
| `hint_rejected` | `reason` | Paid hint rejected (still costs the point) — `"object already fully resolved"` or `"unknown object"` |
| `game_over` | `winner, scores` | Match complete |

### Full turn walkthrough

```python
def send_to_referee(referee_id, msg: dict):
    pynqp2p.send(referee_id, json.dumps(msg))

def poll():
    return [json.loads(raw) for raw in pynqp2p.receive_all()]
```

1. Choose a position, send a flip request:
   ```python
   send_to_referee(referee_id, {"type": "flip", "team": "alpha", "pos": "B3"})
   ```
2. Wait — a **human referee physically flips the real card**. This takes a
   moment; don't assume it's flipped until you get the next message.
3. You'll receive a broadcast (sent to **both** teams, always, regardless of
   whose turn it is):
   ```python
   {"type": "card_revealed", "pos": "B3"}
   ```
   Notice: **no class label is included.** Run your own vision model on
   your own camera frame to identify what's there.
4. Run your detection, log it locally — nothing sent yet.
5. If this was your first flip this turn: choose your second position,
   repeat steps 1–4 for it.
6. **Compare your two detections yourself.** Decide: do they match?
7. Submit your combined claim in one message:
   ```python
   send_to_referee(referee_id, {
       "type": "report_result", "team": "alpha",
       "pos1": "B3", "pos2": "D5",
       "cls1": "dog", "cls2": "dog",
       "claim": "match",   # or "no_match"
   })
   ```
8. You'll get back exactly one of `match`, `no_match`, or `invalid` (see the
   table above).

### Alternative: flip both cards in one request

```python
send_to_referee(referee_id, {"type": "flip_both", "team": "alpha", "pos1": "B3", "pos2": "D5"})
```

Validation is **atomic**: if either position is invalid (already matched,
off-grid, or the same position twice), you get a single `invalid` reply and
**neither** card is revealed — nothing to undo, just fix the bad position
and resend. On success you get two separate `card_revealed` broadcasts, then
everything from "Run your detection" onward is identical to the step-by-step
above.

### Complete minimal turn loop

```python
import pynqp2p, json, time

pynqp2p.register("192.168.1.100:5000", "bootcamp2024")
MY_TEAM = "alpha"
REFEREE_ID = "aa:bb:cc:dd:ee:ff"  # given to you by the RC team

def send(msg):
    pynqp2p.send(REFEREE_ID, json.dumps(msg))

def poll():
    return [json.loads(raw) for raw in pynqp2p.receive_all()]

board_memory = {}  # pos -> class, built from every card_revealed you see

def wait_for(*types, timeout=125):
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in poll():
            if msg["type"] == "card_revealed":
                # runs for EVERY flip, yours or theirs
                board_memory[msg["pos"]] = run_your_model(get_camera_frame())
            if msg["type"] in types:
                return msg
        time.sleep(0.2)
    raise TimeoutError(f"no {types} within {timeout}s")

while True:
    turn_msg = wait_for("your_turn", "game_over")
    if turn_msg["type"] == "game_over":
        print("Game over:", turn_msg)
        break

    pos1 = choose_a_position(board_memory)   # your own strategy logic
    send({"type": "flip", "team": MY_TEAM, "pos": pos1})
    wait_for("card_revealed")                # already updates board_memory[pos1]
    cls1 = board_memory[pos1]

    pos2 = choose_a_position(board_memory, exclude=pos1)
    send({"type": "flip", "team": MY_TEAM, "pos": pos2})
    wait_for("card_revealed")
    cls2 = board_memory[pos2]

    claim = "match" if cls1 == cls2 else "no_match"
    send({
        "type": "report_result", "team": MY_TEAM,
        "pos1": pos1, "pos2": pos2, "cls1": cls1, "cls2": cls2, "claim": claim,
    })
    result = wait_for("match", "no_match")
    print(result)
    # loop continues: if "match", you'll get another your_turn; if "no_match",
    # you'll get "wait" until it's your turn again.
```

## 13. Rescue Modules

For office hours. Each module below is a working, drop-in piece for a team
stuck on **one specific piece** — not the whole solution. Use judgment on
when to reach for these — a team that hasn't attempted the piece at all yet
benefits more from a nudge and a re-read of
[Section 11](#11-whats-given-vs-what-you-build) than from working code.
These are for teams that have genuinely tried and are burning match-day time
on one specific wall, not a first resort.

### Module A — Connection & wire protocol wrapper

**Unblocks:** "we don't know how to actually send/receive messages" or "our
JSON keeps getting malformed."

**Does NOT solve:** what to put in those messages, when to send them, or
what to do with what comes back — that's the turn loop (Module E) and
strategy (Module D).

```python
import pynqp2p, json

class RefereeClient:
    def __init__(self, server, key, referee_id, team, board_id=None):
        pynqp2p.register(server, key)
        self.referee_id = referee_id
        self.team = team
        self.board_id = board_id or pynqp2p.get_id()

    def send(self, message: dict):
        pynqp2p.send(self.referee_id, json.dumps(message))

    def poll(self):
        """Drain and parse every queued message. A malformed line is logged
        and skipped rather than raised -- one bad line shouldn't kill your loop."""
        messages = []
        for raw in pynqp2p.receive_all():
            try:
                messages.append(json.loads(raw))
            except json.JSONDecodeError:
                print(f'[referee] skipping malformed message: {raw!r}')
        return messages

client = RefereeClient(SERVER, BROKER_KEY, REFEREE_ID, TEAM_NAME)
```

### Module B — Vision detection call

**Unblocks:** "we can talk to the referee fine, but we can't reliably turn a
camera frame into a class name."

**Does NOT solve:** cropping the right grid cell out of a full frame (that's
your board's physical geometry), or what confidence threshold is right for
your camera/lighting. This module assumes you already have a cropped image
of just one cell.

```python
import numpy as np

def best_box_in_crop(crop, score_thresh):
    """`run` is your DPU overlay's inference call -- returns boxes/scores/classes.
    Returns (class_name, score) for the single highest-confidence detection,
    or None if nothing cleared score_thresh."""
    boxes, scores, classes = run(crop, score_thresh=score_thresh)
    if not scores.any():
        return None
    idx = int(np.argmax(scores))
    return class_names[int(classes[idx])], float(scores[idx])

def detect_with_fallback(crop, thresholds=(0.5, 0.1, 0.05)):
    """Tries progressively lower confidence thresholds rather than giving up
    after one miss -- a real object at low confidence beats no answer at all."""
    for threshold in thresholds:
        result = best_box_in_crop(crop, threshold)
        if result is not None:
            return result
    return 'unknown', 0.0
```

### Module C — Board memory tracking

**Unblocks:** "we keep forgetting what the opponent already revealed" or
"we're re-detecting the same position every time we see it."

**Does NOT solve:** strategy (Module D) or the turn loop itself (Module E).

```python
board_memory = {}       # pos -> class name, built from every card_revealed seen
matched_positions = set()

def on_card_revealed(pos, detect_fn):
    """Call this for EVERY card_revealed message, regardless of whose turn it
    was or whose flip caused it -- this is the free intel the game's shared-
    visibility rule gives you. Don't only track your own flips."""
    if pos not in board_memory:
        cls, score = detect_fn(pos)
        board_memory[pos] = cls
    return board_memory[pos]

def on_match_confirmed(pos1, pos2):
    matched_positions.add(pos1)
    matched_positions.add(pos2)

def on_no_match(pos1, pos2, true_cls1, true_cls2):
    """true_cls1/true_cls2 come from the referee's no_match response and are
    still golden truth on the wire for now, but deliberately UNUSED here --
    self-correcting board_memory from the answer key defeats the point of
    the vision challenge. If your model misread a card, catch it by letting
    that position get re-observed naturally (e.g. via a future
    card_revealed), not by reading the answer off a wrong guess."""
```

### Module D — Position-choice strategy (baseline)

**Unblocks:** "we have working detection and memory but our strategy is
just guessing randomly" or "we don't know how to prioritize known pairs."

**Does NOT solve:** anything smarter than the obvious greedy rule below —
this is intentionally a floor, not a ceiling. Teams should feel free (and
are encouraged) to improve on it; that's a legitimate scoring lever.

```python
def choose_pair(board_memory, matched_positions, all_positions):
    """Priority: (1) a pair we already know matches, (2) a known class paired
    with something unrevealed (worth exploring since we already have half a
    pair), (3) two fresh unrevealed positions."""
    by_cls = {}
    for pos, cls in board_memory.items():
        if pos in matched_positions:
            continue
        by_cls.setdefault(cls, []).append(pos)

    for positions in by_cls.values():
        if len(positions) >= 2:
            return positions[0], positions[1]

    unrevealed = [p for p in all_positions if p not in board_memory]
    known_pos = next((v[0] for v in by_cls.values()), None)
    if known_pos is not None and unrevealed:
        return known_pos, unrevealed[0]
    if len(unrevealed) >= 2:
        return unrevealed[0], unrevealed[1]

    # Board fully revealed but nothing pairs up -- only happens after a
    # misdetection. Retry two unmatched revealed positions so the referee's
    # next authoritative no_match response can correct us further.
    unmatched_revealed = [p for p in board_memory if p not in matched_positions]
    return unmatched_revealed[0], unmatched_revealed[1]
```

### Module E — Full turn loop (state machine)

**Unblocks:** "everything above works in isolation but we can't wire it
together into an actual game loop" — this is for a team that's out of time,
not out of ideas. Handing this out gives them a complete, working player.

**Does NOT solve:** anything — this wires together Modules A–D as written
above plus a placeholder `run_your_model`. A team using this as-is is
running the reference strategy, not their own; encourage them to swap in
their own `choose_pair` once they're unblocked.

```python
import time

def run_your_model(pos):
    """Replace with your own crop + detect_with_fallback call (Module B)."""
    raise NotImplementedError

def wait_for(client, *types, timeout=125):
    """125s, not 120 -- a little slack over the referee's own 120s turn
    window so you don't time yourself out one message early."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in client.poll():
            if msg['type'] == 'card_revealed':
                on_card_revealed(msg['pos'], run_your_model)
            if msg['type'] in types:
                return msg
        time.sleep(0.2)
    raise TimeoutError(f'no {types} within {timeout}s')

all_positions = [f'{chr(ord("A")+r)}{c+1}' for r in range(GRID_ROWS) for c in range(GRID_COLS)]

while True:
    turn_msg = wait_for(client, 'your_turn', 'game_over')
    if turn_msg['type'] == 'game_over':
        print('Game over:', turn_msg)
        break

    pos1, pos2 = choose_pair(board_memory, matched_positions, all_positions)

    client.send({'type': 'flip_both', 'team': TEAM_NAME, 'pos1': pos1, 'pos2': pos2})
    wait_for(client, 'card_revealed')
    wait_for(client, 'card_revealed')
    cls1, cls2 = board_memory[pos1], board_memory[pos2]

    claim = 'match' if cls1 == cls2 else 'no_match'
    client.send({
        'type': 'report_result', 'team': TEAM_NAME,
        'pos1': pos1, 'pos2': pos2, 'cls1': cls1, 'cls2': cls2, 'claim': claim,
    })
    result = wait_for(client, 'match', 'no_match')
    if result['type'] == 'match':
        matched_positions.update([pos1, pos2])
        # your_turn will come again automatically -- loop continues
    else:
        on_no_match(pos1, pos2, result['cls1'], result['cls2'])
        # a 'wait' message will arrive next; loop back to wait_for('your_turn', ...)
```

### Module F — Hint request + response handling

**Unblocks:** "we don't know how to ask for a hint" or "we got a hint back
and don't know what to do with it."

**Does NOT solve:** reading the digit images themselves (that's meant to be
solved visually or with a small classifier on your side). This module only
shows the request/response mechanics.

**Best practice:** call `request_hint` while it's still the opponent's
turn (`wait` state), not during your own — it costs nothing extra, and the
response arrives before your `your_turn` instead of eating your 120-second
clock. Requesting mid-turn still works, it's just slower for you.

```python
def request_hint(client, obj_name):
    client.send({'type': 'hint_request', 'team': TEAM_NAME, 'object': obj_name})

def wait_for_hint_response(client, timeout=5.0):
    """Only meaningful if you requested during your OWN turn -- a hint
    requested while waiting resolves silently later, right before your next
    your_turn, so there's nothing to poll for here. No response within the
    timeout means silent refusal (score <= 0, or you've already used both
    your hint attempts this match) -- NOT an error, don't retry in a loop.
    On acceptance, returns the two base64-encoded PNG digit images (row,
    then column) -- not text."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in client.poll():
            if msg['type'] == 'hint_response':
                return msg['row_digit_png_base64'], msg['col_digit_png_base64']
            if msg['type'] == 'hint_rejected':
                print(f"hint rejected: {msg['reason']}")   # still cost you a point
                return None
        time.sleep(0.2)
    return None   # silently refused, no cost
```

### Module G — Pre-game riddle + free hint handling

**Unblocks:** "we don't know how to catch the pre-game riddle or free hint
messages."

**Does NOT solve:** solving the riddle itself (LLM call, your side). This
module only shows how to catch and assemble the messages.

```python
def wait_for_pregame_riddle(client, timeout=125):
    """Sent once, identically to both teams, at the very start of the
    pre-game window. No response message exists for this -- whichever team
    calls out the correct answer first tells the human operator out loud."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in client.poll():
            if msg['type'] == 'pregame_riddle':
                return msg['riddle']
        time.sleep(0.2)
    return None

def collect_free_hint_fragments(client, timeout=65):
    """Fragments arrive as separate messages, in no guaranteed order --
    collect by index and only assemble once every 0..total-1 has arrived."""
    fragments = {}
    total = None
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in client.poll():
            if msg['type'] == 'free_hint_fragment':
                fragments[msg['index']] = msg['text']
                total = msg['total']
                if total is not None and len(fragments) == total:
                    return fragments
        time.sleep(0.2)
    return fragments   # possibly incomplete if the window ran out
```

### Module H — MAC discovery / join_competition

**Unblocks:** "the operator has to type our MAC in by hand every match and
we'd rather it auto-fill."

**Does NOT solve:** anything gameplay-related — this is pure convenience for
the operator, entirely optional, and never affects scoring.

```python
def join_competition(client, master_id, secret):
    """Call once, right after connecting. Requires the RC team's MASTER_ID
    (doesn't change between rounds) and the secret shown at your team's
    registration."""
    lobby_id = f'{master_id}-lobby'
    pynqp2p.send(lobby_id, json.dumps({
        'type': 'join', 'team': client.team, 'mac': client.board_id, 'secret': secret,
    }))

# right after RefereeClient(...) succeeds:
join_competition(client, MASTER_ID, TEAM_SECRET)
```

## 14. Glossary

| Term | Meaning |
|---|---|
| Arena Agent / "the referee" | The `gridmind-referee` software validating one live match |
| Master | The tournament orchestrator — registration, scheduling, Grand Final |
| Claim | Your board's own `match`/`no_match` assertion about two flipped cards |
| Golden answer key | The referee's private, authoritative record of what's actually on each card |
| Streak | Consecutive correct matches within the same turn |
| Response-time tier | The speed-based scoring bonus/penalty described in [Section 6](#6-scoring) |
| Puzzle race | The pre-game riddle that decides turn order |
| Free hint | The shared, non-competitive pre-game text hint |
| Paid hint | The mid-match, per-team, point-costing hint |
| `pynqp2p` | The board-to-board messaging library all wire traffic goes through |
| Genesis | The optional, purely cosmetic simulated robot arm layer |

## 15. FAQ

**Q: Our score changed on what looked like "nothing happening" (a timeout or
decline). Is that a bug?**
No — the response-time tier applies even to timeouts and declines now (see
[Section 6](#6-scoring)).

**Q: Can we mix `flip` and `flip_both` in the same turn?**
No — pick one per turn. `flip_both` only works as your turn's first action.

**Q: What happens if our vision model misdetects a card?**
You'll get a `no_match` (or a wrong `match` claim penalty). The response
still includes the true classes on the wire for now, but don't build your
strategy around it — the reference client deliberately ignores it, and a
future referee update is expected to stop sending it. Catch a misdetection
by re-observing that position yourself the next time it comes up, not by
reading the answer off a wrong guess.

**Q: Does Genesis affect our score if a call fails?**
No. Genesis is 100% cosmetic — a failed or skipped Genesis call never
affects your match in any way.

**Q: We played a match against a team called "Referee Bot" — is that real?**
Yes — that's Practice Mode, an operator-triggered way to validate your
client against the real referee without needing a second team. It plays
by the exact same rules and wire protocol as a real opponent (it just
always reports truthfully). Ask an RC Team member to set one up for you
if you want to test before your first real match; it never affects
tournament standings either way.

**Q: We never received the pre-game riddle or free hint — what happened?**
Nothing is sent until **both** teams for your match have connected — check
you've actually clicked Connect (and so has your opponent). If you're both
connected and it still hasn't shown up, tell the operator: they have a
Resend/Restart control for exactly this.

**Q: Where do we ask if something in this doc seems wrong?**
Flag it to the RC Team directly — don't guess, and don't trust an older
planning doc, slide, or Confluence page over this one.
