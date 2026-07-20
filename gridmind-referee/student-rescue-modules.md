# GridMind — Stuck-Team Rescue Modules

For office hours. Each module below is a working, drop-in piece pulled directly
from the reference client (`PYNQ_302-Referee_Match_Client.ipynb`), for handing
to a team stuck on **one specific piece** — not the whole solution. Hand out
only the module that matches where they're actually stuck; each one says
explicitly what it does *not* solve for them, so the rest of the challenge
stays theirs.

Use judgment on when to hand these out — a team that hasn't attempted the piece
at all yet benefits more from a nudge and a re-read of
`student-implementation-guide.md` than from working code. These are for teams
that have genuinely tried and are burning match-day time on one specific wall,
not a first resort.

---

## Module A — Connection & wire protocol wrapper

**Unblocks:** "we don't know how to actually send/receive messages" or
"our JSON keeps getting malformed."

**Does NOT solve:** what to put in those messages, when to send them, or what
to do with what comes back — that's the turn loop (Module E) and strategy
(Module D).

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

---

## Module B — Vision detection call

**Unblocks:** "we can talk to the referee fine, but we can't reliably turn a
camera frame into a class name."

**Does NOT solve:** cropping the right grid cell out of a full frame (that's
your board's physical geometry — the reference notebook's
`crop_grid_cell`/perspective-transform code is grid-layout-specific and
deliberately not included here), or what confidence threshold is right for
your camera/lighting. This module assumes you already have a cropped image of
just one cell.

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

---

## Module C — Board memory tracking

**Unblocks:** "we keep forgetting what the opponent already revealed" or
"we're re-detecting the same position every time we see it."

**Does NOT solve:** strategy (what to do with the memory once you have it —
Module D) or the turn loop itself (Module E).

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
    GOLDEN TRUTH -- overwrite your own (possibly wrong) detection with them so
    future turns don't keep retrying a pair your vision model misread."""
    board_memory[pos1] = true_cls1
    board_memory[pos2] = true_cls2
```

---

## Module D — Position-choice strategy (baseline)

**Unblocks:** "we have working detection and memory but our strategy is just
guessing randomly" or "we don't know how to prioritize known pairs."

**Does NOT solve:** anything smarter than the obvious greedy rule below — this
is intentionally a floor, not a ceiling. Teams should feel free (and are
encouraged) to improve on it; that's a legitimate scoring lever.

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

---

## Module E — Full turn loop (state machine)

**Unblocks:** "everything above works in isolation but we can't wire it
together into an actual game loop" — this is for a team that's out of time,
not out of ideas. Handing this out gives them a complete, working player.

**Does NOT solve:** anything — this wires together Modules A-D as written
above plus a placeholder `run_your_model`. A team using this as-is is running
the reference strategy, not their own; encourage them to swap in their own
`choose_pair` once they're unblocked.

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

---

## Module F — Hint request + response handling

**Unblocks:** "we don't know how to ask for a hint" or "we got a hint back
and don't know what to do with it."

**Does NOT solve:** reading the digit images themselves (that's meant to be
solved visually or with a small classifier on your side — see
`student-implementation-guide.md`). This module only shows the
request/response mechanics, not the digit-reading.

```python
def request_hint(client, obj_name):
    client.send({'type': 'hint_request', 'team': TEAM_NAME, 'object': obj_name})

def wait_for_hint_response(client, timeout=5.0):
    """No response within the timeout means silent refusal (score <= 0, or
    you've already used both your hint attempts this match) -- NOT an error,
    don't retry in a loop. On acceptance, returns the two base64-encoded PNG
    digit images (row, then column) -- not text."""
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

---

## Module G — Pre-game riddle + free hint handling

**Unblocks:** "we don't know how to catch the pre-game riddle or free hint
messages."

**Does NOT solve:** solving the riddle itself (LLM call, your side) or
decoding the QR images (Module B-style vision work, your side). This module
only shows how to catch and assemble the messages.

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

def collect_free_hint_fragments(client, timeout=125):
    """Fragments arrive as separate messages, in no guaranteed order --
    collect by index and only assemble once every 0..total-1 has arrived."""
    fragments = {}
    total = None
    deadline = time.time() + timeout
    while time.time() < deadline:
        for msg in client.poll():
            if msg['type'] == 'free_hint_fragment':
                fragments[msg['index']] = msg['qr_png_base64']  # decode this QR yourself
                total = msg['total']
                if total is not None and len(fragments) == total:
                    return fragments
        time.sleep(0.2)
    return fragments   # possibly incomplete if the window ran out
```

---

## Module H — MAC discovery / join_competition

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
