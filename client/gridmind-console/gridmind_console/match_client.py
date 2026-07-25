"""Match client -- console port of notebook Section 9, unchanged logic.
"""
import threading
import queue
import time

from . import detection
from . import mnist_hint
from . import hints
from . import led


class Stage:
    """Explicit lifecycle stages, replacing free-form status prose with a
    fixed, always-visible sequence. Pregame stages run once per match;
    PLAY_*/WAIT_* alternate every turn depending on whose turn it is."""
    JOIN = 'join'
    RIDDLE_RECEIVED = 'riddle_received'
    RIDDLE_PROCESSED = 'riddle_processed'
    RIDDLE_PRINTED = 'riddle_printed'
    FREE_HINT_RECEIVED = 'free_hint_received'
    FREE_HINT_PROCESSED = 'free_hint_processed'
    FREE_HINT_READY = 'free_hint_ready'
    PLAY_FLIP = 'play_flip'
    PLAY_WAITING = 'play_waiting'
    PLAY_DETECTING = 'play_detecting'
    PLAY_COMPARING = 'play_comparing'
    PLAY_RESULT = 'play_result'
    WAIT_WAITING = 'wait_waiting'
    WAIT_DETECTING = 'wait_detecting'
    WAIT_LOGGING = 'wait_logging'
    GAME_OVER = 'game_over'


PREGAME_STAGES = [
    (Stage.JOIN, 'Join'),
    (Stage.RIDDLE_RECEIVED, 'Riddle Received'),
    (Stage.RIDDLE_PROCESSED, 'Riddle Processed'),
    (Stage.RIDDLE_PRINTED, 'Riddle Printed'),
    (Stage.FREE_HINT_RECEIVED, 'Hint Received'),
    (Stage.FREE_HINT_PROCESSED, 'Hint Processed'),
    (Stage.FREE_HINT_READY, 'Hint Ready'),
]
PLAY_STAGES = [
    (Stage.PLAY_FLIP, 'Flip'),
    (Stage.PLAY_WAITING, 'Waiting'),
    (Stage.PLAY_DETECTING, 'Detecting'),
    (Stage.PLAY_COMPARING, 'Comparing'),
    (Stage.PLAY_RESULT, 'Result'),
]
WAIT_STAGES = [
    (Stage.WAIT_WAITING, 'Waiting'),
    (Stage.WAIT_DETECTING, 'Detecting'),
    (Stage.WAIT_LOGGING, 'Logging'),
]


class MatchClient:
    POLL_INTERVAL_SECONDS = 0.3
    HINT_WAIT_TIMEOUT_SECONDS = 5.0
    # Matches the referee's PHYSICAL_FLIP_OFFSET (game_state.rs / data/game_config.json)
    # -- the fixed overhead the scoring tiers already assume for a physical flip +
    # camera re-capture. Tune to your actual physical setup's flip speed if it differs.
    PHYSICAL_FLIP_DELAY_SECONDS = 20

    def __init__(self, referee_client, on_update=None):
        self.client = referee_client
        self.on_update = on_update or (lambda: None)
        self.lock = threading.Lock()

        self._stop_event = threading.Event()
        self._pause_event = threading.Event()
        self._thread = None
        self._reveal_queue = queue.Queue()
        self._reveal_thread = None

        self.stage = Stage.JOIN
        self.play_mode = False       # Wait Mode by default -- won't act on its own turn until enabled
        self.teams = []
        self.total_pairs = 0
        self.match_number = 0        # local count of game_start messages received on this connection
        self.turn_number = 0         # local action-cycle count; server flip_num is stored separately
        self.current_turn = None     # active turn record shown in the UI
        self.turn_history = []       # completed/current turn records for review
        self.reveal_history = []     # every reveal, including opponent turns
        self.confirmed_matches = []  # authoritative match messages from the server
        self.selected_positions = []  # the two cards output for our current flip_both
        self.unavailable_positions = set()  # locally exclude positions rejected by the server
        self.server_truth_positions = set()  # positions corrected/confirmed by authoritative replies
        self.board_memory = {}       # pos -> class name, from every card_revealed seen
        self.matched_positions = set()
        self.pending = []            # this turn's own flips: [{'pos', 'cls', 'score'}, ...]
        self.awaiting_positions = set()  # our own outstanding flip request(s), to tell them apart from the opponent's
        self.pending_hint_object = None  # queued by queue_hint(), sent at the start of our next turn
        self.my_turn = False
        self.active_team = None
        self.scores = {}
        self.pairs_remaining = None
        self.last_hint = None
        self.last_revealed_pos = None      # set the instant a card_revealed arrives, cleared once detection runs
        self.last_hint_row_png_base64 = None  # from hint_response -- shown in the status panel either way
        self.last_hint_col_png_base64 = None
        self.last_hint_position = None   # auto-decoded from the above via MNIST, e.g. 'C5' -- None if decode failed
        self.last_hint_object = None     # which object the pending/last hint request was for
        self.pregame_riddle = None       # from pregame_riddle -- auto-solved via the LLM below
        self.pregame_riddle_answer = None  # LLM's best-guess answer, or None if solving failed
        self.free_hint_fragments = {}    # index -> fragment text, filled in as free_hint_fragment arrives
        self.free_hint_total = None
        self.free_hint_text = None       # assembled once every fragment 0..total-1 has arrived
        self.free_hint_item = None       # decoded object name, via the LLM solver
        self.free_hint_direction = None  # decoded quadrant ('top left', etc.), via the same solver
        self.free_hint_solve_error = None  # set if the LLM solve step failed or timed out
        self.game_over = False
        self.winner = None
        self.robot_id = None          # from game_start: which Genesis arm is ours (0=red, 1=blue)
        self.genesis_team_id = None   # from game_start: "team_red"/"team_blue" for pynqsim.join_competition()
        self.genesis_url = None       # from game_start: pynqsim server base URL
        self.genesis_sim = None       # live pynqsim.SimulationClient once connected this match, else None
        self.log = []                # raw {'direction', 'message'} entries for the transparency panel

    # -- lifecycle -----------------------------------------------------
    def start(self):
        if self._thread is not None:
            raise RuntimeError('Match client already started.')
        self._stop_event.clear()
        self._pause_event.clear()
        self._reveal_queue = queue.Queue()
        self._reveal_thread = threading.Thread(target=self._run_reveal_worker, daemon=True)
        self._reveal_thread.start()
        self._thread = threading.Thread(target=self._run_loop, daemon=True)
        self._thread.start()

    def pause(self):
        if self.my_turn and self.pending:
            print('[match] WARNING: pausing mid-turn does not pause the referee\'s 120s turn timer.')
        self._pause_event.set()

    def resume(self):
        self._pause_event.clear()

    def stop(self):
        self._stop_event.set()
        self._reveal_queue.put(None)
        if self._thread is not None:
            self._thread.join(timeout=2.0)
        if self._reveal_thread is not None:
            self._reveal_thread.join(timeout=2.0)
        self._thread = None
        self._reveal_thread = None

    # -- externally-triggered actions -----------------------------------
    def set_play_mode(self, enabled):
        """Toggle autonomous play. Turning it on while it's already our turn (and we
        haven't flipped anything yet) kicks off this turn immediately in a background
        thread, rather than waiting for a future your_turn that may not come for a while."""
        with self.lock:
            self.play_mode = enabled
            should_start_now = enabled and self.my_turn and not self.pending and not self.awaiting_positions
        if should_start_now:
            threading.Thread(target=self._start_turn, daemon=True).start()

    def queue_hint(self, obj):
        """Send immediately. The referee queues requests received while we wait
        and resolves them before the next your_turn, outside our turn clock."""
        obj = str(obj).strip()
        if not obj:
            raise ValueError('Hint object cannot be empty.')
        with self.lock:
            self.pending_hint_object = obj
            self.last_hint_object = obj
        self._log('send', {'type': 'hint_request', 'team': self.client.team, 'object': obj})
        self.client.request_hint(obj)
        self.on_update()

    def _solve_pregame_riddle_background(self, riddle_text):
        """Solve off the poll thread so slow LLM I/O never stops referee messages."""
        try:
            result = hints.solve_pregame_riddle_with_llm(riddle_text)
            with self.lock:
                if self.pregame_riddle != riddle_text or self._stop_event.is_set():
                    return
                self.pregame_riddle_answer = result['answer']
                self.stage = Stage.RIDDLE_PROCESSED
            self.on_update()
            fallback_note = ' (plain-text fallback)' if result.get('used_fallback') else ''
            print(f'>>> PRE-GAME RIDDLE ANSWER{fallback_note}: {result["answer"]} <<<')
            with self.lock:
                if self.pregame_riddle == riddle_text and not self._stop_event.is_set():
                    self.stage = Stage.RIDDLE_PRINTED
            self.on_update()
        except Exception as exc:
            print(f'[match] could not auto-solve the pre-game riddle: {exc}')

    def _solve_free_hint_background(self, hint_text):
        """Solve off the poll thread so slow LLM I/O never stops referee
        messages -- mirrors _solve_pregame_riddle_background above."""
        try:
            result = hints.solve_free_hint(hint_text)
            with self.lock:
                if self.free_hint_text != hint_text or self._stop_event.is_set():
                    return
                self.free_hint_item = result['item'] or None
                self.free_hint_direction = result['direction'] or None
                self.stage = Stage.FREE_HINT_READY
            self.on_update()
            print(f'>>> FREE HINT DECODED: item={self.free_hint_item!r}, quadrant={self.free_hint_direction!r} <<<')
        except Exception as exc:
            with self.lock:
                if self.free_hint_text == hint_text and not self._stop_event.is_set():
                    self.free_hint_solve_error = str(exc)
                    self.stage = Stage.FREE_HINT_READY
            self.on_update()
            print(f'[match] could not auto-solve the free hint: {exc}')

    def _begin_turn_record(self, active_team, mine, flip_num=None):
        """Record the server-announced active team and preserve the server's
        optional flip_num without inventing fields in any wire message."""
        with self.lock:
            self.turn_number += 1
            self.active_team = active_team
            self.current_turn = {
                'turn_number': self.turn_number,
                'active_team': active_team,
                'mine': bool(mine),
                'flip_num': flip_num,
                'selected': [],
                'reveals': [],
                'result': None,
            }
            self.turn_history.append(self.current_turn)

    def _finish_current_turn(self, result):
        with self.lock:
            if self.current_turn is not None:
                self.current_turn['result'] = dict(result)

    # -- autonomous turn logic --------------------------------------------
    def _start_turn(self):
        if not self.play_mode:
            return
        with self.lock:
            self.stage = Stage.PLAY_FLIP
        self.on_update()
        self._auto_flip_turn()

    def _auto_flip_turn(self):
        try:
            pos1, pos2 = self._choose_pair()
            self._flip_both(pos1, pos2)
        except RuntimeError as exc:
            print(f'[match] auto-flip failed: {exc}')
        self.on_update()

    def _choose_pair(self):
        """Prefer a guaranteed known pair, then pair a known position with an
        unrevealed one, then two fresh unrevealed positions. If the board is
        fully revealed but nothing pairs up by our own detection (only
        possible after a misdetection -- see the no_match self-correction
        above), retry two already-revealed-but-unmatched positions so the
        referee's next authoritative response can correct us further.

        When exploring blind (no known pair or known+unrevealed shortcut
        available), unrevealed candidates are ordered to try the free
        hint's decoded quadrant first -- it's the only signal available for
        an otherwise-blind flip.
        """
        with self.lock:
            board_memory = dict(self.board_memory)
            unavailable = set(self.matched_positions) | set(self.unavailable_positions)
            hinted_quadrant = self.free_hint_direction

        by_cls = {}
        for pos, cls in board_memory.items():
            if pos in unavailable or cls == 'unknown':
                continue
            by_cls.setdefault(cls, []).append(pos)
        for positions in by_cls.values():
            if len(positions) >= 2:
                return positions[0], positions[1]

        unrevealed = [
            detection.pos_name(row, col)
            for row in range(detection.GRID_ROWS) for col in range(detection.GRID_COLS)
            if detection.pos_name(row, col) not in board_memory and detection.pos_name(row, col) not in unavailable
        ]
        if hinted_quadrant:
            unrevealed.sort(key=lambda pos: detection._quadrant_for_position(pos) != hinted_quadrant)
        known_pos = next(iter(values[0] for values in by_cls.values()), None)
        if known_pos is not None and unrevealed:
            return known_pos, unrevealed[0]
        if len(unrevealed) >= 2:
            return unrevealed[0], unrevealed[1]

        unmatched_revealed = [pos for pos in board_memory if pos not in unavailable]
        if len(unmatched_revealed) < 2:
            raise RuntimeError('Fewer than two available positions remain; waiting for game_over.')
        return unmatched_revealed[0], unmatched_revealed[1]

    def _flip_both(self, pos1, pos2):
        with self.lock:
            if not self.my_turn:
                raise RuntimeError('Not your turn.')
            if self.pending or self.awaiting_positions:
                raise RuntimeError('Already flipped this turn.')
            if pos1 == pos2:
                raise RuntimeError('Cannot flip the same position twice.')
            self.selected_positions = [pos1, pos2]
            self.awaiting_positions = {pos1, pos2}
            if self.current_turn is not None:
                self.current_turn['selected'] = [pos1, pos2]
        self._log('send', {'type': 'flip_both', 'team': self.client.team, 'pos1': pos1, 'pos2': pos2})
        self.client.flip_both(pos1, pos2)
        with self.lock:
            self.stage = Stage.PLAY_WAITING
        self.on_update()
        self._genesis_flip_card(pos1)
        self._genesis_flip_card(pos2)

    # -- Genesis (optional, cosmetic only) --------------------------------
    def _connect_genesis(self):
        """Best-effort: joins Genesis's competition-mode scene as our
        assigned team so _genesis_flip_card/_genesis_end_turn below can
        animate our arm. Any failure (pynqsim not installed, server
        unreachable, already joined) is logged and swallowed, never
        raised -- Genesis is purely cosmetic and never gets a vote in the
        real match, which is decided entirely by report_result."""
        self.genesis_sim = None
        if not (self.genesis_team_id and self.genesis_url):
            return
        try:
            from pynqsim import SimulationClient
            from urllib.parse import urlparse
            parsed = urlparse(self.genesis_url)
            sim = SimulationClient(parsed.hostname, port=parsed.port or 9002)
            sim.join_competition(team_id=self.genesis_team_id)
            self.genesis_sim = sim
            print(f'[genesis] joined as {self.genesis_team_id} at {self.genesis_url}')
        except Exception as exc:
            print(f'[genesis] connection skipped (cosmetic only, match unaffected): {exc}')
            self.genesis_sim = None

    def _genesis_flip_card(self, pos):
        """Mirrors a real flip onto the simulated arm, purely for visual
        effect -- the referee's own report_result claim is what actually
        decides the match either way."""
        if self.genesis_sim is None:
            return
        try:
            row, col = detection.parse_pos(pos)
            self.genesis_sim.flip_card(row, col)
        except Exception as exc:
            print(f'[genesis] flip_card({pos}) failed (cosmetic only): {exc}')

    def _genesis_end_turn(self):
        if self.genesis_sim is None:
            return
        try:
            self.genesis_sim.end_turn()
        except Exception as exc:
            print(f'[genesis] end_turn failed (cosmetic only): {exc}')

    def _disconnect_genesis(self):
        if self.genesis_sim is None:
            return
        try:
            self.genesis_sim.leave_competition()
        except Exception as exc:
            print(f'[genesis] leave_competition failed (cosmetic only): {exc}')
        self.genesis_sim = None

    # -- background loop --------------------------------------------------
    def _run_loop(self):
        while not self._stop_event.is_set():
            if self._pause_event.is_set():
                time.sleep(self.POLL_INTERVAL_SECONDS)
                continue
            try:
                for message in self.client.poll():
                    self._handle_message(message)
            except Exception as exc:
                print(f'[match] poll error: {type(exc).__name__}: {exc}')
            time.sleep(self.POLL_INTERVAL_SECONDS)

    def _handle_message(self, message):
        self._log('recv', message)
        message_type = message.get('type')

        if message_type == 'game_start':
            self._disconnect_genesis()
            with self.lock:
                self.match_number += 1
                self.teams = list(message['teams'])
                self.total_pairs = int(message['total_pairs'])
                self.turn_number = 0
                self.current_turn = None
                self.turn_history = []
                self.reveal_history = []
                self.confirmed_matches = []
                self.selected_positions = []
                self.unavailable_positions = set()
                self.server_truth_positions = set()
                self.board_memory = {}
                self.matched_positions = set()
                self.pending = []
                self.awaiting_positions = set()
                self.pending_hint_object = None
                self.last_hint = None
                self.last_hint_row_png_base64 = None
                self.last_hint_col_png_base64 = None
                self.last_hint_position = None
                self.last_hint_object = None
                self.my_turn = False
                self.active_team = None
                self.scores = {team: 0 for team in self.teams}
                self.pairs_remaining = self.total_pairs
                self.game_over = False
                self.winner = None
                self.robot_id = message.get('robot_id')
                self.genesis_team_id = message.get('genesis_team_id')
                self.genesis_url = message.get('genesis_url')
            self._connect_genesis()
        elif message_type == 'your_turn':
            self._begin_turn_record(self.client.team, mine=True, flip_num=message.get('flip_num'))
            with self.lock:
                self.my_turn = True
                self.pending = []
                self.awaiting_positions = set()
                self.selected_positions = []
                if self.pending_hint_object:
                    self.last_hint = f'No response for {self.pending_hint_object!r} (server silently refused it).'
                    self.pending_hint_object = None
                self.stage = Stage.PLAY_FLIP
            self.on_update()
            self._start_turn()
            return
        elif message_type == 'wait':
            self._begin_turn_record(message['active_team'], mine=False)
            with self.lock:
                self.my_turn = False
                self.selected_positions = []
                self.stage = Stage.WAIT_WAITING
            self._genesis_end_turn()
        elif message_type == 'card_revealed':
            self._queue_card_revealed(message['pos'])
        elif message_type == 'invalid':
            with self.lock:
                rejected_positions = list(self.selected_positions)
                self.unavailable_positions.update(rejected_positions)
                self.awaiting_positions = set()
                self.pending = []
                self.selected_positions = []
                self.stage = Stage.PLAY_FLIP
                if self.current_turn is not None:
                    self.current_turn.setdefault('invalid_attempts', []).append({
                        'positions': rejected_positions,
                        'reason': message['reason'],
                    })
                should_retry = self.play_mode and self.my_turn and not self.game_over
            print(f"[match] flip rejected: {message['reason']}")
            if should_retry:
                threading.Thread(target=self._auto_flip_turn, daemon=True).start()
        elif message_type == 'match':
            with self.lock:
                self.matched_positions.update([message['pos1'], message['pos2']])
                self.unavailable_positions.update([message['pos1'], message['pos2']])
                self.server_truth_positions.update([message['pos1'], message['pos2']])
                if message.get('cls'):
                    self.board_memory[message['pos1']] = message['cls']
                    self.board_memory[message['pos2']] = message['cls']
                self.scores = dict(message['scores'])
                self.pairs_remaining = message['remaining']
                self.pending = []
                confirmed = {
                    'match_number': self.match_number,
                    'turn_number': self.turn_number,
                    'pos1': message['pos1'],
                    'pos2': message['pos2'],
                    'class': message.get('cls'),
                    'scorer': message.get('scorer'),
                }
                self.confirmed_matches.append(confirmed)
                if self.current_turn is not None:
                    self.current_turn['result'] = {'type': 'match', **confirmed}
            self._flash_status_led(led.STATUS_LED_COLORS['match_found'])
        elif message_type == 'no_match':
            with self.lock:
                # The referee deliberately doesn't send the real classes at
                # pos1/pos2 here -- a misdetected pair has to be caught by
                # re-observing it yourself, not by the referee handing you
                # the answer on a wrong guess. Scoring is unaffected either
                # way: it's always decided server-side against the grid's
                # real answer key, never from what a client reports.
                self.scores = dict(message['scores'])
                self.pending = []
                if self.current_turn is not None:
                    self.current_turn['result'] = {
                        'type': 'no_match',
                        'pos1': message['pos1'], 'pos2': message['pos2'],
                    }
            self._flash_status_led(led.STATUS_LED_COLORS['no_match'])
        elif message_type == 'pregame_riddle':
            # Sent to both teams during the pre-game window. Auto-solved via
            # the halo Strix LLM below -- tell the human referee out loud if
            # you're first to answer correctly. There is no wire message for
            # submitting an answer, judging is entirely manual.
            with self.lock:
                self.pregame_riddle = message['riddle']
                self.pregame_riddle_answer = None
                self.free_hint_fragments = {}
                self.free_hint_total = None
                self.free_hint_text = None
                self.free_hint_item = None
                self.free_hint_direction = None
                self.free_hint_solve_error = None
                self.stage = Stage.RIDDLE_RECEIVED
            self.on_update()
            threading.Thread(
                target=self._solve_pregame_riddle_background,
                args=(self.pregame_riddle,),
                daemon=True,
                name='pregame-riddle-solver',
            ).start()
        elif message_type == 'free_hint_fragment':
            # One shared, non-competitive hint, split into plain-text
            # fragments -- assemble every index 0..total-1 yourself.
            assembled_text = None
            with self.lock:
                self.free_hint_fragments[message['index']] = message['text']
                self.free_hint_total = message['total']
                self.stage = Stage.FREE_HINT_RECEIVED
                if len(self.free_hint_fragments) == self.free_hint_total:
                    ordered = [self.free_hint_fragments[i] for i in range(self.free_hint_total)]
                    self.free_hint_text = ' '.join(ordered)
                    self.free_hint_item = None
                    self.free_hint_direction = None
                    self.free_hint_solve_error = None
                    self.stage = Stage.FREE_HINT_PROCESSED
                    assembled_text = self.free_hint_text
            self.on_update()
            if assembled_text is not None:
                # Decode off the poll thread -- _choose_pair() below reads
                # free_hint_direction on every autonomous flip once this
                # finishes, not just for display.
                threading.Thread(
                    target=self._solve_free_hint_background,
                    args=(assembled_text,),
                    daemon=True,
                    name='free-hint-solver',
                ).start()
        elif message_type == 'hint_response':
            # Row and column arrive as small digit photos, not text --
            # decode them ourselves via the on-board MNIST classifier
            # (same model used by the Manual Hint Override tool) rather
            # than relying on a human to read them off the screen.
            self.pending_hint_object = None
            self.last_hint_row_png_base64 = message['row_digit_png_base64']
            self.last_hint_col_png_base64 = message['col_digit_png_base64']
            try:
                row_digit = mnist_hint.decode_digit_png_base64(self.last_hint_row_png_base64)
                col_digit = mnist_hint.decode_digit_png_base64(self.last_hint_col_png_base64)
                # Digits are 1-indexed (row A=1, col 1=1) to match position
                # naming directly -- pos_name() itself is 0-indexed.
                self.last_hint_position = detection.pos_name(row_digit - 1, col_digit - 1)
                self.last_hint = f'Paid hint resolved: row={row_digit}, col={col_digit} -> {self.last_hint_position}'
                if self.last_hint_object:
                    # Feed straight into board_memory -- _choose_pair()'s
                    # existing tiered logic (guaranteed match / hedge /
                    # explore) picks this up automatically on the very
                    # next call, no special-casing needed there. Same
                    # pattern the no_match self-correction already uses.
                    self.board_memory[self.last_hint_position] = self.last_hint_object
            except Exception as exc:
                self.last_hint_position = None
                self.last_hint = f'row/col digit images received, but MNIST decode failed ({exc}) -- see images below'
        elif message_type == 'hint_rejected':
            self.pending_hint_object = None
            self.last_hint = f"rejected: {message['reason']}"
        elif message_type == 'game_over':
            with self.lock:
                self.game_over = True
                self.winner = message['winner']
                self.scores = message['scores']
                self.stage = Stage.GAME_OVER
            self._disconnect_genesis()

        self.on_update()

    def _queue_card_revealed(self, pos):
        """Capture ownership immediately, then let the poll loop keep draining
        while one serialized worker handles physical delay and camera access."""
        with self.lock:
            is_mine = pos in self.awaiting_positions
            if is_mine:
                self.awaiting_positions.discard(pos)
            turn_record = self.current_turn
            item = {
                'pos': pos,
                'mine': is_mine,
                'match_number': self.match_number,
                'turn_number': self.turn_number,
                'active_team': self.active_team,
                'turn_record': turn_record,
            }
            self.stage = Stage.PLAY_DETECTING if is_mine else Stage.WAIT_DETECTING
            self.last_revealed_pos = pos
        print(f'>>> PHYSICAL REFEREE: flip the real card at position {pos} now <<<')
        self.on_update()
        self._reveal_queue.put(item)

    def _run_reveal_worker(self):
        while not self._stop_event.is_set():
            item = self._reveal_queue.get()
            if item is None:
                self._reveal_queue.task_done()
                break
            try:
                while self._pause_event.is_set() and not self._stop_event.is_set():
                    time.sleep(self.POLL_INTERVAL_SECONDS)
                if not self._stop_event.is_set():
                    self._process_card_revealed(item)
            except Exception as exc:
                print(f"[match] reveal processing failed for {item['pos']}: {type(exc).__name__}: {exc}")
            finally:
                self._reveal_queue.task_done()

    def _process_card_revealed(self, item):
        pos = item['pos']
        is_mine = item['mine']
        with self.lock:
            if self.current_turn is item['turn_record'] and not self.game_over:
                self.stage = Stage.PLAY_DETECTING if is_mine else Stage.WAIT_DETECTING
        self.on_update()
        time.sleep(self.PHYSICAL_FLIP_DELAY_SECONDS)
        with self.lock:
            if item['match_number'] != self.match_number:
                return  # stale queued reveal from a previous scheduled match
            if self.last_revealed_pos == pos:
                self.last_revealed_pos = None
        try:
            cls, score = detection.detect_position(pos, show=True)
        except Exception as exc:
            print(f'[detect] {pos} failed: {type(exc).__name__}: {exc}')
            cls, score = 'unknown', 0.0

        should_submit = False
        with self.lock:
            if pos not in self.server_truth_positions:
                self.board_memory[pos] = cls
            reveal = {
                'match_number': item['match_number'],
                'turn_number': item['turn_number'],
                'active_team': item['active_team'],
                'mine': is_mine,
                'pos': pos,
                'class': cls,
                'score': float(score),
                'timestamp': time.time(),
            }
            self.reveal_history.append(reveal)
            turn_record = item['turn_record']
            if turn_record is not None:
                turn_record['reveals'].append(reveal)
            is_current_turn = self.current_turn is turn_record
            if is_mine and is_current_turn:
                self.pending.append({'pos': pos, 'cls': cls, 'score': score})
                should_submit = len(self.pending) == 2 and not self.game_over
                if not should_submit and not self.game_over:
                    self.stage = Stage.PLAY_DETECTING
            elif is_current_turn and not self.game_over:
                self.stage = Stage.WAIT_LOGGING

        self.on_update()
        if is_mine and should_submit:
            self._submit_result()
        elif not is_mine and self.current_turn is item['turn_record'] and not self.game_over:
            with self.lock:
                if not self.game_over:
                    self.stage = Stage.WAIT_WAITING
            self.on_update()

    def _submit_result(self):
        with self.lock:
            self.stage = Stage.PLAY_COMPARING
            first, second = self.pending[0], self.pending[1]
        self.on_update()
        # Two failed detections must never become a false "unknown == unknown" match.
        claim = 'match' if first['cls'] != 'unknown' and first['cls'] == second['cls'] else 'no_match'
        self._log('send', {
            'type': 'report_result', 'team': self.client.team,
            'pos1': first['pos'], 'pos2': second['pos'],
            'cls1': first['cls'], 'cls2': second['cls'], 'claim': claim,
        })
        self.client.report_result(first['pos'], second['pos'], first['cls'], second['cls'], claim)
        with self.lock:
            self.stage = Stage.PLAY_RESULT

    def _flash_status_led(self, color, duration_seconds=0.6):
        """Briefly shows `color` on the status LED, then restores whatever
        the persistent status would otherwise be (via on_update()). Runs
        off the poll thread so match/no_match handling itself stays fast."""
        def worker():
            led.set_status_led(color)
            time.sleep(duration_seconds)
            self.on_update()
        threading.Thread(target=worker, daemon=True, name='status-led-flash').start()

    def _log(self, direction, message):
        with self.lock:
            self.log.append({'direction': direction, 'message': message})
            self.log = self.log[-200:]
