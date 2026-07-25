"""Referee wire-protocol client -- console port of notebook Section 8, unchanged."""
import json

import pynqp2p


class RefereeClient:
    def __init__(self, server, key, referee_id, team, master_id=None, board_id=None):
        pynqp2p.register(server, key)
        self.referee_id = referee_id
        self.master_id = master_id
        self.team = team
        self.board_id = board_id or pynqp2p.get_id()

    def send(self, message):
        pynqp2p.send(self.referee_id, json.dumps(message))

    def poll(self):
        """Drain and parse every queued message. A malformed line is logged
        and skipped rather than raised, since one bad line should never take
        down the match loop."""
        messages = []
        for raw in pynqp2p.receive_all():
            try:
                messages.append(json.loads(raw))
            except json.JSONDecodeError:
                print(f'[referee] skipping malformed message: {raw!r}')
        return messages

    def flip_both(self, pos1, pos2):
        self.send({'type': 'flip_both', 'team': self.team, 'pos1': pos1, 'pos2': pos2})

    def report_result(self, pos1, pos2, cls1, cls2, claim):
        self.send({
            'type': 'report_result', 'team': self.team,
            'pos1': pos1, 'pos2': pos2, 'cls1': cls1, 'cls2': cls2, 'claim': claim,
        })

    def request_hint(self, obj):
        self.send({'type': 'hint_request', 'team': self.team, 'object': obj})

    def join_competition(self, secret):
        """Self-reports this board's MAC to the Master's lobby, once, right
        after connecting -- lets the operator's match-assign popup auto-fill
        your MAC instead of typing it in by hand. Requires `master_id` (does
        not change between rounds, unlike `referee_id`). Entirely optional:
        skip it and the operator can still enter your MAC manually."""
        if not self.master_id:
            print('[referee] join_competition skipped: no MASTER_ID set.')
            return
        lobby_id = f'{self.master_id}-lobby'
        pynqp2p.send(lobby_id, json.dumps({
            'type': 'join', 'team': self.team, 'mac': self.board_id, 'secret': secret,
        }))
