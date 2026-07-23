# GridMind Manual Demo Walkthrough

Every command below is copy-pasteable `curl`. No scripts — you type each step live, so you can pause anywhere and explain what's happening. Assumes a fresh broker + Master + 2 Arenas already running (commands at the bottom if not).

Ports used below: broker `35050`, Master web UI `38800`. Adjust if yours differ.

---

## Part 1 — Operator: Register teams

Open **http://localhost:38800/operator** in a browser and narrate this, or run it via curl:

```bash
curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" \
  -d '{"name":"alpha","students":["Priya","Jamal"]}'

curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" \
  -d '{"name":"beta","students":["Wren"]}'
```

Refresh the operator/scoreboard pages — both teams now appear, auto-balanced into pools, with a live-building schedule underneath. Optional: show the "move to other pool" button working before closing.

```bash
curl -s -X POST http://localhost:38800/api/close-registration
```

Watch the Master's terminal log — it prints `Next match for arena 1: alpha vs beta` and the operator console's Schedule view now shows that row as clickable (green "Ready").

## Part 2 — Operator: Assign the match

Before assigning the match, each team can self-report its MAC instead of the operator typing it by hand -- send a `join` for each team (the secrets shown here match the registration view; substitute the real ones from your own run). The `id` below (`master-referee-lobby`) is always `<the master's --id>-lobby` -- if you started the master with a different `--id`, use that value instead:

```bash
curl -s -X POST -d 'key=demokey&id=master-referee-lobby&message={"type":"join","team":"alpha","mac":"aa:aa:aa:aa:aa:aa","secret":"<alpha's secret>"}' \
  http://localhost:35050/send
curl -s -X POST -d 'key=demokey&id=master-referee-lobby&message={"type":"join","team":"epsilon","mac":"bb:bb:bb:bb:bb:bb","secret":"<epsilon's secret>"}' \
  http://localhost:35050/send
```

In the operator console, click the Ready row — if both joins landed, the MAC fields are already filled in (green border); otherwise type them manually as before. Either way, pick who solved the puzzle race:

```bash
curl -s -X POST http://localhost:38800/api/start-match -H "Content-Type: application/json" \
  -d '{"winner":"alpha","team_a_mac":"aa:aa:aa:aa:aa:aa","team_b_mac":"bb:bb:bb:bb:bb:bb"}'
```

The Master immediately sends `assign_match` to Arena 1, which loads the grid and pushes `game_start` + the first `your_turn` to alpha's board (`aa:aa:...`). Confirm it arrived:

```bash
curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://localhost:35050/receive_all
```

You should see (2 lines):
```
{"type":"game_start","teams":["alpha","beta"],"total_pairs":4}
{"type":"your_turn","flip_num":1}
```

The grid in play is `gridmind-referee/example_grid.json`:
```
A1=dog       A2=aeroplane   A3=person   A4=bottle
B1=person    B2=dog         B3=bottle   B4=aeroplane
```

## Part 3 — Simulate Team Alpha's turn (the active team)

**flip_both** — reveal two positions in one round-trip (the faster protocol):

```bash
curl -s -X POST -d "key=demokey&id=aa:aa:aa:aa:aa:aa&message={\"type\":\"flip_both\",\"team\":\"alpha\",\"pos1\":\"A1\",\"pos2\":\"A2\"}" \
  http://localhost:35050/send
```

Both teams receive `card_revealed` for A1 and A2 — check beta's queue too, to show the "both teams see every flip" rule live:

```bash
curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://localhost:35050/receive_all
curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
```

**report_result** — alpha reports what it saw. A1=dog, A2=aeroplane — not a match, so claim `no_match`:

```bash
curl -s -X POST -d 'key=demokey&id=aa:aa:aa:aa:aa:aa&message={"type":"report_result","team":"alpha","pos1":"A1","pos2":"A2","cls1":"dog","cls2":"aeroplane","claim":"no_match"}' \
  http://localhost:35050/send

curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://localhost:35050/receive_all
```

Response: `no_match` (deliberately doesn't echo back the real classes -- a misdetection has to be caught by re-observing the position, not by reading the referee's answer key), then `wait` (turn passes to beta).

## Part 4 — Simulate a correct match (streak scoring)

Alpha's turn is over — it's beta's turn now (single-flip protocol this time, to show both work). Beta flips B2 (dog) then, having seen A1=dog from Part 3, flips A1 again knowing it's a match:

```bash
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"flip","team":"beta","pos":"B2"}' http://localhost:35050/send
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"flip","team":"beta","pos":"A1"}' http://localhost:35050/send
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"report_result","team":"beta","pos1":"B2","pos2":"A1","cls1":"dog","cls2":"dog","claim":"match"}' http://localhost:35050/send

curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
```

Response: `match` (+1 point, `remaining: 3`), then **beta keeps the turn** (correct matches don't end your turn — this is what makes streaks possible). Do a second correct match in a row to show streak scoring (1st match = 1pt, 2nd consecutive = +2 = 3pts total):

```bash
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"flip_both","team":"beta","pos1":"A4","pos2":"B3"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"report_result","team":"beta","pos1":"A4","pos2":"B3","cls1":"bottle","cls2":"bottle","claim":"match"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
```

## Part 5 — Simulate a wrong match (penalty + turn ends)

Beta guesses wrong on purpose to show the penalty path — flips A3 (person) and B1 (person)... actually let's force a genuine wrong claim: flip A2 (aeroplane) and B4 (aeroplane) but claim `no_match` incorrectly, or flip two non-matching cards and claim `match`:

```bash
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"flip_both","team":"beta","pos1":"A3","pos2":"B4"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
curl -s -X POST -d 'key=demokey&id=bb:bb:bb:bb:bb:bb&message={"type":"report_result","team":"beta","pos1":"A3","pos2":"B4","cls1":"person","cls2":"aeroplane","claim":"match"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=bb:bb:bb:bb:bb:bb" http://localhost:35050/receive_all
```

Response: score drops by the flat wrong-match penalty (2 points by default, regardless of how fast the claim came in), turn passes to alpha immediately (both cards flip back — nothing is removed from play, they can be flipped again later).

## Part 6 — Paid hint (accepted, then rejected)

It's alpha's turn now. Request a hint for an object that still has an unrevealed position (e.g. "person" — A3 is revealed but not matched, B1 is untouched):

```bash
curl -s -X POST -d 'key=demokey&id=aa:aa:aa:aa:aa:aa&message={"type":"hint_request","team":"alpha","object":"person"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://localhost:35050/receive_all
```

Response: `hint_response` with two base64-encoded digit-image PNGs (`row_digit_png_base64`, `col_digit_png_base64`) — not text; row/column of the still-unrevealed position, costs 1 point, deducted immediately. Now try requesting a hint for an object that's already fully matched (e.g. "dog", both positions already found in Part 4) — this shows the rejection path:

```bash
curl -s -X POST -d 'key=demokey&id=aa:aa:aa:aa:aa:aa&message={"type":"hint_request","team":"alpha","object":"dog"}' http://localhost:35050/send
curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://localhost:35050/receive_all
```

Response: `hint_rejected` (still costs the point — "deducted as soon as requested" per the rules).

## Part 7 — Finish the match

Play out the remaining pairs (A2/B4 = aeroplane, A3/B1 = person) the same way as Part 3/4 to reach `4/4 pairs found`. Once the last pair is confirmed, the arena automatically reports `match_result` to the Master — no manual step needed. Watch it happen live:

```bash
tail -f /tmp/master_show.log
```

You'll see the Master print `[pool 1] match ended, winner: ..., scores: ...`, and — since this is likely the only match in pool 1 (2 teams) — it immediately becomes eligible for the Grand Final if pool 2 also finishes. Refresh the scoreboard to show the standings table update and the Champion screen once the tournament concludes.

---

## Appendix: starting everything from scratch

The p2p broker (`server.py`, `pynqp2p` library) is distributed separately
from this directory — **TODO(havish): fill in where organizers get it for
event day.**

```bash
# Terminal 1 — broker
cd <path-to-broker>
venv/bin/python server.py --host 0.0.0.0 --port 35050 --key demokey

# Terminal 2 — Master (no --config = live registration mode)
cd gridmind-referee
./target/debug/gridmind-referee master --server localhost:35050 --key demokey --id master-referee --web-port 38800

# Terminal 3 — Arena 1
./target/debug/gridmind-referee arena --server localhost:35050 --key demokey --id arena-1-referee --master-id master-referee --arena-num 1

# Terminal 4 — Arena 2 (only needed once pool 2 has a match)
./target/debug/gridmind-referee arena --server localhost:35050 --key demokey --id arena-2-referee --master-id master-referee --arena-num 2
```

Open `http://localhost:38800/operator` and `http://localhost:38800/` (scoreboard) side by side in two browser windows/monitors.
