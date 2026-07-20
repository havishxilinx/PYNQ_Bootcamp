#!/usr/bin/env python3
"""
GridMind demo student bot.

Stands in for a real PYNQ board for demo purposes: talks to a live Arena
Agent using the exact wire protocol documented in student-api-reference.html.
There's no camera or vision model on this dev server, so `detect()` looks
up the golden answer key directly instead of running inference on a camera
frame -- every other part of the exchange (message shapes, turn loop, hint
use) is the real, unmodified protocol round-trip against the actual Rust
referee binary.
"""
import argparse
import json
import time
import requests


class Bot:
    def __init__(self, server, key, my_id, referee_id, team, golden, flip_mode="single"):
        self.server = server
        self.key = key
        self.my_id = my_id
        self.referee_id = referee_id
        self.team = team
        self.golden = golden
        self.flip_mode = flip_mode
        self.queue = []
        self.discovered = {}  # pos -> cls, learned from card_revealed broadcasts
        self.matched = set()

    def send(self, msg):
        payload = json.dumps(msg)
        requests.post(
            f"http://{self.server}/send",
            data={"key": self.key, "id": self.referee_id, "message": payload},
        ).raise_for_status()
        print(f"[{self.my_id}] SEND -> {self.referee_id}: {payload}", flush=True)

    def join_competition(self, master_id, secret):
        """Sends a one-time join announcing this team's MAC to the
        Master's lobby, so the operator's popup can auto-fill it. See
        docs/superpowers/specs/2026-07-13-join-competition-design.html.
        """
        lobby_id = f"{master_id}-lobby"
        payload = json.dumps({
            "type": "join",
            "team": self.team,
            "mac": self.my_id,
            "secret": secret,
        })
        requests.post(
            f"http://{self.server}/send",
            data={"key": self.key, "id": lobby_id, "message": payload},
        ).raise_for_status()
        print(f"[{self.my_id}] JOIN -> {lobby_id}: {payload}", flush=True)

    def _pump(self):
        r = requests.get(
            f"http://{self.server}/receive_all",
            data={"key": self.key, "id": self.my_id},
        )
        r.raise_for_status()
        for raw in r.text.split("\n"):
            if not raw:
                continue
            msg = json.loads(raw)
            print(f"[{self.my_id}] RECV: {raw}", flush=True)
            self.queue.append(msg)

    def wait_for(self, *types, timeout=90):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if not self.queue:
                self._pump()
                if not self.queue:
                    time.sleep(0.3)
                    continue
            msg = self.queue.pop(0)
            if msg["type"] == "card_revealed":
                pos = msg["pos"]
                self.discovered[pos] = self.golden[pos]
            if msg["type"] == "match":
                self.matched.add(msg["pos1"])
                self.matched.add(msg["pos2"])
            if msg["type"] in types:
                return msg
        raise TimeoutError(f"[{self.my_id}] no {types} within {timeout}s")

    def try_wait_for(self, *types, timeout):
        try:
            return self.wait_for(*types, timeout=timeout)
        except TimeoutError:
            return None

    def choose_pair(self):
        by_cls = {}
        for pos, cls in self.discovered.items():
            if pos in self.matched:
                continue
            by_cls.setdefault(cls, []).append(pos)
        for positions in by_cls.values():
            if len(positions) >= 2:
                return positions[0], positions[1]

        unrevealed = [p for p in self.golden if p not in self.discovered and p not in self.matched]
        if by_cls and unrevealed:
            known_pos = next(iter(v[0] for v in by_cls.values()))
            return known_pos, unrevealed[0]
        return unrevealed[0], unrevealed[1]

    def flip(self, pos):
        self.send({"type": "flip", "team": self.team, "pos": pos})
        while True:
            msg = self.wait_for("card_revealed", "invalid")
            if msg["type"] == "invalid":
                self.matched.add(pos)
                return False
            if msg["pos"] == pos:
                return True
            # card_revealed for some other position -- a broadcast from a
            # concurrent/backlogged opponent move, not our own flip. Already
            # recorded in self.discovered by wait_for; keep waiting for ours.

    def flip_both(self, pos1, pos2):
        self.send({"type": "flip_both", "team": self.team, "pos1": pos1, "pos2": pos2})
        seen = set()
        while len(seen) < 2:
            msg = self.wait_for("card_revealed", "invalid")
            if msg["type"] == "invalid":
                self.matched.add(pos1)
                return False
            if msg["pos"] in (pos1, pos2):
                seen.add(msg["pos"])
        return True

    def maybe_request_hint(self, hint_object):
        self.send({"type": "hint_request", "team": self.team, "object": hint_object})
        resp = self.try_wait_for("hint_response", "hint_rejected", timeout=5)
        if resp is None:
            print(f"[{self.my_id}] hint silently refused (no response within 5s)", flush=True)
        else:
            print(f"[{self.my_id}] hint outcome: {resp}", flush=True)

    def play(self, hint_object=None):
        print(f"[{self.my_id}] waiting for game to start...", flush=True)
        first_turn = True
        while True:
            msg = self.wait_for("game_start", "your_turn", "wait", "game_over")
            if msg["type"] == "game_over":
                print(f"[{self.my_id}] GAME OVER: {msg}", flush=True)
                return
            if msg["type"] == "your_turn":
                break

        while True:
            if first_turn and hint_object:
                self.maybe_request_hint(hint_object)
                first_turn = False

            while True:
                pos1, pos2 = self.choose_pair()
                if self.flip_mode == "double":
                    ok = self.flip_both(pos1, pos2)
                else:
                    ok = self.flip(pos1)
                    if ok:
                        ok = self.flip(pos2)
                if ok:
                    break
            cls1 = self.discovered[pos1]
            cls2 = self.discovered[pos2]

            claim = "match" if cls1 == cls2 else "no_match"
            self.send(
                {
                    "type": "report_result",
                    "team": self.team,
                    "pos1": pos1,
                    "pos2": pos2,
                    "cls1": cls1,
                    "cls2": cls2,
                    "claim": claim,
                }
            )
            result = self.wait_for("match", "no_match")
            print(f"[{self.my_id}] result: {result['type']}", flush=True)

            nxt = self.wait_for("your_turn", "wait", "game_over")
            if nxt["type"] == "game_over":
                print(f"[{self.my_id}] GAME OVER: {nxt}", flush=True)
                return
            if nxt["type"] == "wait":
                nxt = self.wait_for("your_turn", "game_over")
                if nxt["type"] == "game_over":
                    print(f"[{self.my_id}] GAME OVER: {nxt}", flush=True)
                    return


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="127.0.0.1:5000")
    ap.add_argument("--key", default="bootcamp2024")
    ap.add_argument("--team", required=True)
    ap.add_argument("--id", required=True, dest="my_id")
    ap.add_argument("--referee-id", required=True)
    ap.add_argument("--grid", required=True)
    ap.add_argument("--hint-object", default=None)
    ap.add_argument("--flip-mode", choices=["single", "double"], default="single")
    ap.add_argument("--secret", default=None, help="team secret from registration; required to send a join")
    ap.add_argument("--master-id", default=None, help="master's board ID, used to derive the lobby ID for join_competition")
    args = ap.parse_args()

    golden = json.load(open(args.grid))["positions"]
    bot = Bot(args.server, args.key, args.my_id, args.referee_id, args.team, golden, flip_mode=args.flip_mode)
    if args.secret and args.master_id:
        bot.join_competition(args.master_id, args.secret)
    bot.play(hint_object=args.hint_object)


if __name__ == "__main__":
    main()
