# GridMind: 3-Machine Distributed Demo

Shows the real network topology GridMind will run on at the event — a central broker + tournament orchestrator on one machine, with independent Arena processes on separate physical machines, all talking over plain HTTP. This mirrors the actual competition-day layout (just with 2 Arena machines instead of the full 4-machine event setup — broker+Master combined here onto one machine since we only have 3 total).

**Machines used for this demo:**
| Role | Machine | IP |
|---|---|---|
| Broker + Master (orchestrator + both web UIs) | Your machine | (local) |
| Arena 1 | Strix Halo box 1 | `172.20.166.171` |
| Arena 2 | Strix Halo box 2 | `172.20.167.210` |

---

## Talking points (for narrating the demo)

- "GridMind isn't one program — it's several independent processes that only know about each other through one shared message broker. That's exactly how the real competition will run: one broker machine, one Master (tournament brain), and one Arena machine per physical game station."
- "The Arena machines don't know anything about tournament structure, pools, or scheduling — they just wait for an assignment, referee one match using the rules engine, and report the result back. All the tournament logic — who plays whom, standings, the Grand Final — lives entirely in the Master."
- "Student boards will talk directly to their Arena's machine over the same broker — I'll simulate that by sending the exact same messages a real board would send, by hand, from the command line."
- (After the round-trip below) "That flip request went out from this laptop, over the network, to the Arena process running on the Strix box across the room, got scored by the rules engine there, and the result came straight back to this screen — that's the full path a real match takes."

---

## Exact steps I ran to set this up (so you can redo it yourself)

### 1. Verify SSH connectivity to both machines

```bash
ssh amd@172.20.166.171 "uname -a"
ssh amd@172.20.167.210 "uname -a"
```
Both are Ubuntu 24.04, x86_64 — same architecture as the build machine, so a compiled binary transfers directly (no need to install Rust on them).

### 2. One-time fix: switch off OpenSSL, use rustls

The referee binary originally linked against `libssl.so.1.1`, which doesn't exist on Ubuntu 24.04 (that OpenSSL version is EOL). Fixed in `Cargo.toml`:
```toml
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
```
Then rebuilt:
```bash
cd gridmind-referee
~/.cargo/bin/cargo build
```
This is already committed (`3f7da6a`) — you don't need to redo this, just make sure you're using a binary built *after* that commit. Verify with:
```bash
ldd target/debug/gridmind-referee | grep ssl   # should print nothing
```

### 3. Copy the binary + grid file to both Arena machines

```bash
ssh amd@172.20.166.171 "mkdir -p ~/gridmind-demo"
ssh amd@172.20.167.210 "mkdir -p ~/gridmind-demo"

scp gridmind-referee/target/debug/gridmind-referee gridmind-referee/example_grid.json \
  amd@172.20.166.171:~/gridmind-demo/
scp gridmind-referee/target/debug/gridmind-referee gridmind-referee/example_grid.json \
  amd@172.20.167.210:~/gridmind-demo/

ssh amd@172.20.166.171 "chmod +x ~/gridmind-demo/gridmind-referee"
ssh amd@172.20.167.210 "chmod +x ~/gridmind-demo/gridmind-referee"
```

### 4. Find your machine's real IP (the one the remote machines can reach)

```bash
hostname -I | awk '{print $1}'
```
This demo used `10.26.16.41` — substitute your own. **Do not use `localhost`** for the remote machines' `--server` flag; they need your machine's actual address.

### 5. Start the broker (on your machine)

The p2p broker (`server.py`, `pynqp2p` library) is distributed separately
from this directory — **TODO(havish): fill in where organizers get it for
event day.** Once you have it:

```bash
cd <path-to-broker>
python3 server.py --host 0.0.0.0 --port 35050 --key demokey
```

### 6. Start the Master (on your machine, separate terminal)

```bash
cd gridmind-referee
./target/debug/gridmind-referee master --server localhost:35050 --key demokey --id master-referee --web-port 38800
```
Open `http://localhost:38800/operator` and `http://localhost:38800/` in two browser windows.

### 7. Start Arena 1 (on Strix box 1) — **cwd matters, the grid file path is relative**

```bash
ssh amd@172.20.166.171
cd ~/gridmind-demo
./gridmind-referee arena --server 10.26.16.41:35050 --key demokey --id arena-1-referee --master-id master-referee --arena-num 1
```
(Leave this running in its own terminal/SSH session — don't background it with a bare `&` from a one-shot `ssh host "cmd &"` unless you also redirect stdin with `< /dev/null`, otherwise the SSH session can hang waiting on the remote shell.)

### 8. Start Arena 2 (on Strix box 2), same pattern

```bash
ssh amd@172.20.167.210
cd ~/gridmind-demo
./gridmind-referee arena --server 10.26.16.41:35050 --key demokey --id arena-2-referee --master-id master-referee --arena-num 2
```

### 9. Register teams + close registration (operator console or curl)

```bash
curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" -d '{"name":"alpha","students":["Priya"]}'
curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" -d '{"name":"beta","students":["Wren"]}'
curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" -d '{"name":"delta","students":["Noor"]}'
curl -s -X POST http://localhost:38800/api/register-team -H "Content-Type: application/json" -d '{"name":"epsilon","students":["Kai"]}'
curl -s -X POST http://localhost:38800/api/close-registration
```
Use 4 teams (not 2) so both pools get a real match and both Arena machines are actually exercised.

### 10. Assign both matches from the operator console

Click each "Ready" row in the Schedule view, enter any MAC placeholder for each team (e.g. `aa:aa:aa:aa:aa:aa`), pick the puzzle-race winner, submit. This sends the real `assign_match` across the network to whichever Strix box owns that arena.

### 11. Simulate a player — the "hook the audience" moment

From your own machine (or literally any machine that can reach the broker — this is the point: a "player" is just anyone sending the right messages):

```bash
# Send: flip two positions (talking to the Arena's board ID, NOT the student's own MAC)
curl -s -X POST -d 'key=demokey&id=arena-1-referee&message={"type":"flip_both","team":"alpha","pos1":"A1","pos2":"A2"}' \
  http://10.26.16.41:35050/send

# Receive: what the student board gets back (game_start, your_turn, card_revealed x2)
curl -s -X GET -d "key=demokey&id=aa:aa:aa:aa:aa:aa" http://10.26.16.41:35050/receive_all
```
Point at the Master's terminal — you'll see `[arena 1, pool 1] 0/4 pairs, scores: ...` appear the instant that message round-trips through the remote Arena machine and back. That's the whole distributed system working live.

For the full turn-by-turn script (matches, wrong guesses, hints, finishing a match) once this topology is live, see `manual-demo-walkthrough.md` in this same directory — every command in it works unchanged against this 3-machine setup, just point `id=` at `arena-1-referee`/`arena-2-referee` as shown above instead of assuming everything's on one box.

---

## Cleanup after the demo

```bash
# On each Strix machine (Ctrl+C the arena process, or from your machine):
ssh amd@172.20.166.171 "pkill -f gridmind-referee"
ssh amd@172.20.167.210 "pkill -f gridmind-referee"
```
