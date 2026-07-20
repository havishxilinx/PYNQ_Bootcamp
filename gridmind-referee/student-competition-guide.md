# GridMind — Student Competition Guide

Everything you need, from the first line of code to the final match. This guide covers **what to build**, **how the game is played**, and **how scoring works** — all verified directly against the actual referee software that will run on event day, not against any older planning document.

> This guide is fully up to date as of 2026-07-20 — the response-time tier table in Section 4 was tightened up this session to account for a fixed physical-flip allowance; see that section for the current numbers.

---

## 1. What GridMind Is

GridMind is a two-team AI matching-card competition. A grid of face-down cards sits between two teams, each object appearing on exactly two cards. Each team's board runs its own vision model — there's no shared "referee vision system" watching the grid. When a card is revealed, **both teams** get to look at it (with their own camera and their own model) and remember what they saw. On your turn, you pick two cards, your board tells the referee whether they match, and the referee checks your answer against the real, hidden answer key.

**The referee never runs its own vision model and never tells you what's on a card.** It only validates the claim you send it. This is the single most important thing to understand — see [`student-implementation-guide.md`](student-implementation-guide.md) for the full breakdown of what the referee does for you vs. what your team has to build.

---

## 2. Tournament Format

- Teams are split into **two pools**. Within a pool, everyone plays everyone else once (round robin).
- The team with the best record in each pool advances to a single **Grand Final** match.
- **Exception:** if only two teams total are registered, pools are skipped entirely and it goes straight to one Grand-Final-style match.
- There is no elimination round — a bad match doesn't knock you out of your pool.

Standings within a pool are ranked by wins, with total pairs matched as the tiebreaker.

---

## 3. Playing a Match

### The grid and the board

Cards sit face-down on a physical grid (e.g. a 3×5 or similar layout — exact size and object classes are set per event, confirmed by the RC Team). Positions are labeled like `A1`, `B3`, `C5` — a letter for the row, a number for the column.

### Pre-game window

Two things happen here, in order, before the match clock starts:

1. **The riddle race (decides who goes first).** The referee sends both teams the same hard riddle (`pregame_riddle`) with a single-word answer. You get about 2 minutes to solve it with your own LLM. **Judging is entirely manual** — there's no wire message for submitting an answer. Whichever team calls out the correct answer first, out loud, to the human operator, wins and goes first. If nobody gets it, the operator decides (a coin flip, typically).
2. **The free hint (shared, not competitive).** Right after, both teams receive an identical hint — a riddle describing one real object on the grid and its rough quadrant, split into several QR-coded fragments (`free_hint_fragment`) you assemble yourself, the same as scanning printed cards with your camera. This one doesn't affect turn order; it's free intel for both teams equally.

The match itself begins with a `game_start` message naming both teams in turn order (whoever won the riddle race goes first).

### Your turn

1. Pick two grid positions and tell the referee (`flip` twice, or `flip_both` for both positions in one round trip — see the API reference for the exact message shapes).
2. A human referee physically flips the real card(s). You'll get a `card_revealed` broadcast for each position — **with no class label**. Run your own vision model on your own camera frame to figure out what's there.
3. Compare your two detections yourself. Report your claim: `match` or `no_match`.
4. The referee checks your claim against the real answer key:
   - **Correct match:** points awarded, and **your turn continues** — go back to step 1.
   - **Wrong claim, or genuinely no match:** your turn ends. If you were wrong, the referee also tells you the *true* classes of both cards — update your own memory with that, don't keep re-guessing the same wrong pair.
5. **You keep receiving every `card_revealed` broadcast even during the opponent's turn.** Both teams see everything that's ever flipped — this is deliberate. A team that ignores the opponent's turn is throwing away free information.

### Turn timer

You have **120 seconds** from the moment you're handed the turn to act. If you go over, the referee ends your turn for you.

---

## 4. Scoring

### Correct matches — streak bonus

Each match in a row within the same turn is worth one more point than the last:

| Matches in a row this turn | Points for that turn |
|---|---|
| 1 | 1 |
| 2 | 3 (1+2) |
| 3 | 6 (1+2+3) |
| 4 | 10 (1+2+3+4) |

### Wrong match

**−1 point**, and your turn ends immediately.

### Response-time tier — applies to every turn outcome, including timeouts

On top of the above, how fast you acted after being handed the turn adds or subtracts points — **this applies even if you time out or decline**, which is a real behavior worth knowing about:

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

A team that always sits and waits out the full clock is now actively penalized every turn, not just missing out on points.

### Hints

**Paid hints** (mid-match, on-demand): request a hint for a specific object name during your own active turn. It costs **1 point**, deducted immediately, capped at **2 attempts per match**, and only available if your score is currently above 0. You get back **two small digit images** (base64-encoded PNGs) — one for the row number, one for the column number — not text. Read them yourself (row letters map to numbers the same way as everywhere else: A=1, B=2...). If your request is outside those conditions (score too low, already used both attempts, unknown/mistyped object name — matching is case-sensitive), it's either silently ignored (no cost) or rejected with a reason (still costs the point — see the API reference for the exact rejection reasons).

**Free hints** (pre-game, shared): see the Pre-Game Window section above — one shared hint per match, delivered as assembled QR fragments, no cost, not competitive.

---

## 5. What Your Team Needs to Build

Full breakdown: [`student-implementation-guide.md`](student-implementation-guide.md). Short version — you're building:

1. **Vision detection** — turn a camera frame (or a crop of one) into an object class name.
2. **Board memory** — remember every `card_revealed` you've ever seen, including the opponent's flips.
3. **Comparison logic** — decide `match` vs `no_match` from your own two detections.
4. **Position-choice strategy** — this is the actual game. The referee has no opinion on which cells you pick.
5. **Turn loop** — wait for `your_turn`, drive the flip → detect → report cycle, handle `wait` and `game_over`.
6. **Pre-game riddle solving** — an LLM call to solve the single-word riddle (for turn order) fast enough to shout it out first.
7. **Free-hint QR assembly** — decode each `free_hint_fragment`'s QR image and concatenate them in order.
8. **Paid-hint digit reading** — interpret the two digit images `hint_response` sends back.
9. **(Optional)** `join_competition` MAC self-reporting.

If your team gets stuck on any one of these specifically, ask an RC Team member during office hours — there are drop-in reference modules for each piece in [`student-rescue-modules.md`](student-rescue-modules.md) that unblock one part without handing you the whole solution.

---

## 6. Wire Protocol Cheat Sheet

Full reference with exact JSON shapes and a complete worked example: [`student-api-reference.html`](student-api-reference.html). Quick summary of what you send and receive:

**You send:**
| Message | When |
|---|---|
| `flip` / `flip_both` | Choosing card(s) on your turn |
| `report_result` | After detecting both your flipped cards |
| `hint_request` | Optional, only on your active turn |
| `join` (`join_competition`) | Optional, once, right after connecting |

**You receive:**
| Message | Meaning |
|---|---|
| `pregame_riddle` | Pre-game, both teams identically — decides who goes first (judged manually) |
| `free_hint_fragment` | Pre-game, both teams identically — assemble every fragment yourself |
| `game_start` | Match beginning, tells you turn order and total pairs |
| `your_turn` / `wait` | Whose turn it is |
| `card_revealed` | Broadcast to both teams on every physical flip — no class label |
| `invalid` | Your last flip request was rejected — pick a different position |
| `match` / `no_match` | Result of your claim |
| `hint_response` / `hint_rejected` | Hint outcome — two digit images, not text |
| `game_over` | Match complete, winner and final scores |

Connect with `pynqp2p.register(server, key)`, send with `pynqp2p.send(referee_id, json_string)`, and drain your inbox with `pynqp2p.receive_all()` (returns a list, and empties your queue — each message is delivered once).

---

## 7. Getting Connected on Competition Day

1. The RC Team gives you: the broker address, the broker key, your **referee ID** (changes per match — you'll be told the new one before each match), and your team's **master ID** (fixed, doesn't change).
2. At registration, you're given a one-time **secret**. If you fill in `TEAM_SECRET` and `MASTER_ID` in your notebook and connect, your board automatically reports its MAC address to the operator — this just saves the operator from typing it in by hand. It's optional; if you skip it, the operator can enter your MAC manually with zero penalty to you.
3. Once both teams' MACs and a turn order are confirmed, the operator starts your match and you'll get `game_start`.

---

## 8. Official Rules

1. **No human input once the match clock starts.** Your board operates on its own from `game_start` to `game_over`.
2. **Turn time limit is 120 seconds.** Acting faster is rewarded (see the response-time tier above); running out the clock is now actively penalized, not neutral.
3. **A correct match keeps your turn.** Keep flipping until you get a wrong claim or the match ends.
4. **A wrong claim costs 1 point and ends your turn immediately.**
5. **Both teams receive every card reveal, regardless of whose turn it was.** Use that information — it's free.
6. **Paid hints cost 1 point per attempt, capped at 2 per match, and require a positive score.** Free hints (pre-game, shared) cost nothing.
7. **The referee's decision is final.** It validates your claim against the real answer key; it does not take your word for it, and it does not run its own vision model.
8. **Tiebreakers within a pool:** most wins, then most total pairs matched.

---

## 9. If Something Doesn't Match What You Expected

This guide is written against the actual, tested referee software — if something you read elsewhere (an older planning doc, a slide, a previous year's rules) contradicts what's here, **this document and the RC Team win**. If you think you've found a real discrepancy in this guide itself, flag it to the RC Team rather than guessing.
