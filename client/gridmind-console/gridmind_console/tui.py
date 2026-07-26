"""Textual TUI -- console port of notebook Section 10 (Widget GUI), Section
1a's board-clock panel, and Section 12's MNIST/override wiring. Every button
and field from the notebook dashboard has a direct equivalent here; nothing
was dropped, only the live camera/debug image previews (which a terminal
can't render) -- those are written to debug/*.jpg instead, see detection.py.
"""
import json
import sys
import threading
import time
import traceback

import requests
from textual.app import App, ComposeResult
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.widgets import (
    Header, Footer, Static, Input, Button, Label, RadioSet, RadioButton,
    Checkbox, TabbedContent, TabPane, RichLog, Select,
)

from . import board_clock
from . import detection
from . import mnist_hint
from . import hints
from . import led
from .referee_client import RefereeClient
from .match_client import MatchClient, Stage, PLAY_STAGES, WAIT_STAGES, PREGAME_STAGES

STAGE_COLOR_DONE = 'green'
STAGE_COLOR_ACTIVE = 'yellow'
STAGE_COLOR_PENDING = 'grey50'


def render_stage_tracker(stage):
    """Rich-markup equivalent of the notebook's HTML stage-tracker breadcrumb."""
    if stage == Stage.GAME_OVER:
        return '[bold white on purple] Game Over [/]'

    stage_keys = [key for key, _ in PLAY_STAGES]
    if stage in stage_keys:
        stages, label_prefix = PLAY_STAGES, 'Play: '
    else:
        stage_keys = [key for key, _ in WAIT_STAGES]
        if stage in stage_keys:
            stages, label_prefix = WAIT_STAGES, 'Wait: '
        else:
            stages, label_prefix = PREGAME_STAGES, ''

    keys = [key for key, _ in stages]
    current_idx = keys.index(stage) if stage in keys else -1
    chips = []
    for idx, (_, label) in enumerate(stages):
        color = STAGE_COLOR_DONE if idx < current_idx else STAGE_COLOR_ACTIVE if idx == current_idx else STAGE_COLOR_PENDING
        chips.append(f'[bold white on {color}] {label} [/]')
    prefix = f'[bold]{label_prefix}[/bold]' if label_prefix else ''
    return prefix + ' '.join(chips)


def _status_led_color_for_match(match):
    if match is None:
        return None
    if match.game_over:
        if match.winner == match.client.team:
            return led.STATUS_LED_COLORS['won']
        elif match.winner:
            return led.STATUS_LED_COLORS['lost']
        return led.STATUS_LED_COLORS['tied']
    if match.pending_hint_object:
        return led.STATUS_LED_COLORS['hint_pending']
    play_stage_keys = {key for key, _ in PLAY_STAGES}
    wait_stage_keys = {key for key, _ in WAIT_STAGES}
    if match.stage in play_stage_keys:
        return led.STATUS_LED_COLORS['your_turn']
    if match.stage in wait_stage_keys:
        return led.STATUS_LED_COLORS['opponent_turn']
    return led.STATUS_LED_COLORS['connecting']


class _StdoutTee:
    """Redirects print()/stdout writes to both the real stdout and the
    Debug Log RichLog widget. Detection, match_client, mnist_hint, etc.
    all use plain print() for diagnostics (border-marker counts, detection
    misses, exceptions) from background threads -- once the TUI takes
    over the screen, those writes go nowhere visible at all. This makes
    them show up inside the app instead of requiring a second SSH session
    tailing a redirected log file."""

    def __init__(self, app, real_stdout):
        self.app = app
        self.real_stdout = real_stdout
        self._buffer = ''

    def write(self, text):
        self.real_stdout.write(text)
        self._buffer += text
        while '\n' in self._buffer:
            line, self._buffer = self._buffer.split('\n', 1)
            if line:
                self._write_line(line)
        return len(text)

    def _write_line(self, line):
        if threading.current_thread() is self.app._app_thread:
            self._append(line)
        else:
            try:
                self.app.call_from_thread(self._append, line)
            except Exception:
                pass  # app shutting down / no longer running

    def _append(self, line):
        try:
            self.app.query_one('#debug_log', RichLog).write(line)
        except Exception:
            pass

    def flush(self):
        self.real_stdout.flush()

    def isatty(self):
        return False


class GridMindApp(App):
    """Console equivalent of the notebook's Section 10 dashboard."""

    CSS = """
    Screen {
        background: $background;
    }

    Header {
        background: #1d1a1a;
        color: #ffffff;
    }

    TabbedContent > ContentSwitcher {
        padding: 1 2;
    }

    Tabs {
        background: #1d1a1a;
    }

    Tab.-active {
        color: #d71111;
        text-style: bold;
    }

    /* Section headers -- a plain colored label, NOT boxed. Distinct from
       .panel (an actual bordered content/output box) so headers don't get
       stacked box-in-a-box with the content right below them. */
    .section-title {
        text-style: bold;
        color: #d71111;
        margin: 1 0 0 0;
        height: auto;
    }

    /* Descriptive callouts and output logs -- these genuinely benefit from
       standing apart visually as "read this" / "results here" boxes. */
    .panel {
        border: round #4a4a4a;
        padding: 1 2;
        margin-bottom: 1;
        background: $panel;
    }

    /* Match tab's Stage/Status -- the single most important thing on
       screen at a glance, gets its own stronger visual treatment. */
    .status-box {
        border: round #d71111;
        padding: 1 2;
        margin-bottom: 1;
        background: $panel;
        text-style: bold;
    }

    .row {
        height: auto;
        margin-bottom: 1;
    }

    /* One labeled input: a small caption directly above its field, so a
       pre-filled Input (which hides its own placeholder text) still says
       what it is. */
    .field {
        width: 1fr;
        margin-right: 1;
        height: auto;
    }

    .field-label {
        color: $text-muted;
        text-style: none;
        margin: 0 0 0 1;
        height: 1;
    }

    Input {
        width: 1fr;
        margin-right: 1;
    }

    .field Input {
        margin-right: 0;
    }

    Input:focus {
        border: round #d71111;
    }

    Button {
        margin-right: 1;
        min-width: 14;
    }

    RichLog {
        min-height: 8;
        border: round #4a4a4a;
    }
    """

    BINDINGS = [('q', 'quit', 'Quit')]
    TITLE = 'GridMind'

    def __init__(self, config):
        super().__init__()
        self.config = config
        self.match = None
        self._app_thread = None
        self.sub_title = f'Not connected · team {config.team_name!r}'

    # -- layout ----------------------------------------------------------
    def compose(self) -> ComposeResult:
        yield Header()
        with TabbedContent(initial='connect'):
            with TabPane('Connect', id='connect'):
                with VerticalScroll():
                    yield Static('[bold]Board Clock[/bold]', classes='section-title')
                    with Horizontal(classes='row'):
                        yield Input(placeholder='YYYY-MM-DD HH:MM:SS', id='board_time_input')
                        yield Button('Set Board Time', id='set_time_button', variant='warning')
                    with Horizontal(classes='row'):
                        yield Input(placeholder='http://192.168.1.100:5000', id='sync_time_url_input')
                        yield Button('Sync From HTTP', id='sync_time_button', variant='primary')
                    yield Static('', id='board_clock_output', classes='panel')

                    yield Static('[bold]Connection[/bold]', classes='section-title')
                    with Horizontal(classes='row'):
                        with Vertical(classes='field'):
                            yield Label('Broker server', classes='field-label')
                            yield Input(value=self.config.server, id='server_input')
                        with Vertical(classes='field'):
                            yield Label('Broker key', classes='field-label')
                            yield Input(value=self.config.broker_key, id='key_input')
                        with Vertical(classes='field'):
                            yield Label('Referee (arena) ID', classes='field-label')
                            yield Input(value=self.config.referee_id, id='referee_input')
                        with Vertical(classes='field'):
                            yield Label('Master ID', classes='field-label')
                            yield Input(value=self.config.master_id, id='master_input')
                    with Horizontal(classes='row'):
                        with Vertical(classes='field'):
                            yield Label('Team name', classes='field-label')
                            yield Input(value=self.config.team_name, id='team_input')
                        with Vertical(classes='field'):
                            yield Label('Team secret (blank to skip)', classes='field-label')
                            yield Input(value=self.config.team_secret, placeholder='team secret (blank to skip)', id='team_secret_input', password=True)
                        with Vertical(classes='field'):
                            yield Label('Board ID override', classes='field-label')
                            yield Input(value=self.config.board_id_override, placeholder='board ID override', id='board_id_input')
                    with Horizontal(classes='row'):
                        yield Button('Test Connectivity', id='test_connectivity_button', variant='primary')
                        yield Button('Connect', id='connect_button', variant='success')
                        yield Button('Disconnect', id='disconnect_button', variant='error', disabled=True)
                    yield RichLog(id='connect_output', classes='panel', wrap=True)

            with TabPane('Pre-Match Testing', id='pretest'):
                with VerticalScroll():
                    yield Static('[bold]Raw camera capture[/bold] -- no ArUco/YOLO involved, just proves the camera works.\nWrites debug/detection.jpg -- open it in an image viewer to confirm.', classes='panel')
                    yield Button('Capture Test Frame', id='camera_capture_test_button', variant='primary')

                    yield Static(
                        f'[bold]Camera / Grid-Cell Debug[/bold] -- fixed pipeline: {self.config.detection_approach}. '
                        f'Writes debug/alignment.jpg (auto-oriented board) and debug/processed_crop.jpg '
                        f'(oriented 416x416 YOLO input). Never touches match state or the referee.',
                        classes='panel',
                    )
                    with Horizontal(classes='row'):
                        yield Input(value='A1', id='camera_debug_position_input')
                        yield Button('Debug Oriented Cell', id='camera_debug_button', variant='primary')
                    yield RichLog(id='camera_debug_output', classes='panel', wrap=True)

                    yield Static(
                        '[bold]MNIST Digit Debug (self-test)[/bold] -- checks the on-board MNIST classifier '
                        'and its camera independent of the referee. Writes debug/mnist_digit.jpg.',
                        classes='panel',
                    )
                    with Horizontal(classes='row'):
                        yield Select([], id='mnist_camera_select', allow_blank=True, prompt='MNIST camera')
                        yield RadioSet(
                            RadioButton('Live Camera', value=True, id='mnist_source_camera'),
                            RadioButton('Saved Image', id='mnist_source_saved'),
                            id='mnist_digit_source',
                        )
                        yield Select([(str(d), d) for d in range(10)], value=0, id='saved_digit_select')
                    yield Button('Debug MNIST One-Shot', id='mnist_debug_button', variant='primary')
                    yield RichLog(id='mnist_output', classes='panel', wrap=True)

            with TabPane('Match', id='match'):
                with VerticalScroll():
                    with Horizontal(classes='row'):
                        yield Button('Start', id='start_button', variant='success', disabled=True)
                        yield Button('Pause', id='pause_button', disabled=True)
                        yield Button('Resume', id='resume_button', disabled=True)
                        yield Button('Stop', id='stop_button', variant='error', disabled=True)
                    yield Static(
                        'Control: Manual uses the Wait/Play toggle below. Auto switches to Play Mode '
                        'automatically as soon as Start is clicked -- no manual toggle needed.',
                        classes='panel',
                    )
                    yield RadioSet(
                        RadioButton('Manual', value=True, id='control_manual'),
                        RadioButton('Auto', id='control_auto'),
                        id='control_mode',
                    )
                    yield RadioSet(
                        RadioButton('Wait Mode', value=True, id='mode_wait'),
                        RadioButton('Play Mode', id='mode_play'),
                        id='play_mode',
                        disabled=True,
                    )
                    yield Static('[bold]Stage[/bold]', classes='section-title')
                    yield Static('', id='stage_display', classes='status-box')
                    yield Static('[bold]Status[/bold]', classes='section-title')
                    yield Static('Not connected.', id='status_display', classes='status-box')
                    yield Static('', id='board_display')

            with TabPane('Hints', id='hints'):
                with VerticalScroll():
                    yield Static(
                        "Both the pre-game riddle and the free hint auto-solve via the LLM as soon as they "
                        "arrive -- these buttons re-solve manually if needed. The free hint's decoded quadrant "
                        "also biases which unrevealed cells auto-play tries first.",
                        classes='panel',
                    )
                    with Horizontal(classes='row'):
                        yield Button('Solve Riddle (LLM)', id='solve_riddle_button', variant='primary')
                        yield Button('Re-solve Free Hint (LLM)', id='resolve_free_hint_button', variant='primary')

                    yield Static(
                        '[bold]Queue Hint[/bold] -- asks the referee for a paid hint; it decodes automatically '
                        'via the on-board MNIST model as soon as it arrives (see Status). Manual Hint Override '
                        'below is only for when that auto-decode fails or misreads a digit.',
                        classes='panel',
                    )
                    with Horizontal(classes='row'):
                        yield Input(placeholder='e.g. dog', id='hint_object_input')
                        yield Button('Queue Hint', id='hint_button', variant='warning', disabled=True)

                    yield Static('[bold]Manual Hint Override[/bold]', classes='section-title')
                    with Horizontal(classes='row'):
                        yield Input(placeholder='card name, e.g. car', id='override_name_input')
                        yield Button('Start Manual Override', id='override_start_button', variant='warning')
                        yield Button('Capture Row', id='override_capture_button', variant='warning', disabled=True)
                    with Horizontal(classes='row'):
                        yield Button('Show Overrides', id='overrides_show_button', variant='primary')
                        yield Button('Clear Overrides', id='overrides_clear_button', variant='error')
                    yield RichLog(id='hint_output', classes='panel', wrap=True)

            with TabPane('Log', id='log'):
                yield Static('[bold]Wire Messages[/bold]', classes='section-title')
                yield Checkbox('Show raw wire messages', id='show_raw_log_checkbox')
                yield RichLog(id='wire_log', wrap=True)
                yield Static(
                    '[bold]Debug / System Log[/bold] -- everything detection.py, match_client.py, '
                    'etc. print() (border-marker counts, detection misses, exceptions) -- previously '
                    'invisible once the TUI took over the screen.',
                    classes='panel',
                )
                yield RichLog(id='debug_log', wrap=True)
        yield Footer()

    def on_mount(self):
        self._app_thread = threading.current_thread()
        self._real_stdout = sys.stdout
        sys.stdout = _StdoutTee(self, self._real_stdout)
        options = detection.video_devices() or []
        select = self.query_one('#mnist_camera_select', Select)
        select.set_options([(d, d) for d in options])
        if options:
            default = next((d for d in options if d != detection.camera_device), options[0])
            select.value = default
        self.render_status()

    def on_unmount(self):
        sys.stdout = self._real_stdout

    # -- helpers -----------------------------------------------------------
    def _log_to(self, widget_id, text):
        self.query_one(f'#{widget_id}', RichLog).write(text)

    def _clear_log(self, widget_id):
        self.query_one(f'#{widget_id}', RichLog).clear()

    def _report_exception(self, widget_id, exc):
        self._log_to(widget_id, ''.join(traceback.format_exception(type(exc), exc, exc.__traceback__)))

    # -- Connect tab actions -------------------------------------------------
    def action_set_board_time(self):
        self._clear_log_static('board_clock_output')
        value = self.query_one('#board_time_input', Input).value.strip()
        try:
            if not value:
                raise ValueError('Enter a date/time first, e.g. 2026-07-16 09:00:00')
            board_clock.set_board_time(value)
            self.query_one('#board_clock_output', Static).update('Board time set.')
        except Exception as exc:
            self.query_one('#board_clock_output', Static).update(f'[red]{exc}[/red]')

    def action_sync_board_time(self):
        url = self.query_one('#sync_time_url_input', Input).value.strip()
        try:
            if not url:
                raise ValueError('Enter a reachable URL first, e.g. the broker address.')
            board_clock.sync_board_time_from_http(url)
            self.query_one('#board_clock_output', Static).update('Board time synced.')
        except Exception as exc:
            self.query_one('#board_clock_output', Static).update(f'[red]{exc}[/red]')

    def _clear_log_static(self, widget_id):
        self.query_one(f'#{widget_id}', Static).update('')

    def action_test_connectivity(self):
        """Quick, fast-failing check that this board can actually reach the
        broker before attempting Connect -- pynqp2p's own requests calls have
        no timeout, so a genuinely unreachable broker makes Connect hang
        forever instead of failing with a clear error. Run this first."""
        self._clear_log('connect_output')
        server = self.query_one('#server_input', Input).value.strip()
        key = self.query_one('#key_input', Input).value.strip()
        if not server:
            self._log_to('connect_output', 'Enter a Server address first.')
            return
        url = f'http://{server}/ping'
        self._log_to('connect_output', f'Testing connectivity to {url} ...')
        start = time.time()
        try:
            response = requests.post(url, data={'key': key, 'id': 'connectivity-test'}, timeout=8)
            elapsed_ms = int((time.time() - start) * 1000)
            if response.status_code == 200:
                self._log_to('connect_output', f'REACHABLE -- got {response.text!r} in {elapsed_ms}ms. Safe to Connect.')
            elif response.status_code == 401:
                self._log_to('connect_output', 'Reached the broker, but the Key is wrong (401 Unauthorized). Check BROKER_KEY.')
            else:
                self._log_to('connect_output', f'Reached the broker, but got an unexpected response: {response.status_code} {response.text!r}')
        except requests.exceptions.ConnectTimeout:
            elapsed_ms = int((time.time() - start) * 1000)
            self._log_to('connect_output', (
                f'NOT REACHABLE -- connection timed out after {elapsed_ms}ms. '
                'This board cannot reach the broker at all -- check network routing '
                '(relay/tunnel setup) before trying Connect, which will hang the same way.'
            ))
        except requests.exceptions.ConnectionError as exc:
            self._log_to('connect_output', f'NOT REACHABLE -- connection refused or DNS failure: {exc}')
        except Exception as exc:
            self._log_to('connect_output', f'Connectivity test failed: {type(exc).__name__}: {exc}')

    def action_connect(self):
        self._clear_log('connect_output')
        try:
            server = self.query_one('#server_input', Input).value
            key = self.query_one('#key_input', Input).value
            referee_id = self.query_one('#referee_input', Input).value
            master_id = self.query_one('#master_input', Input).value
            team = self.query_one('#team_input', Input).value
            team_secret = self.query_one('#team_secret_input', Input).value
            board_id = self.query_one('#board_id_input', Input).value

            client = RefereeClient(
                server, key, referee_id, team,
                master_id=master_id or None,
                board_id=board_id or None,
            )
            if team_secret:
                client.join_competition(team_secret)
                self._log_to('connect_output', f'Sent join_competition for team {client.team!r} (mac {client.board_id}).')
            self.match = MatchClient(client, on_update=self._on_match_update)
            mnist_hint.match = self.match
            # Start receiving messages immediately, not on the separate Start
            # click below -- join_competition() already satisfies the
            # referee's "team joined" gate at Connect time, so the pregame
            # riddle/free hint can arrive before Start is ever clicked. Every
            # autonomous action is separately gated by play_mode (off by
            # default), so it's safe to listen from Connect onward.
            self.match.start()
            self._log_to('connect_output', f'Connected as board {client.board_id}, team {client.team!r}.')
            self.sub_title = f'Connected · board {client.board_id} · team {client.team!r}'
            self.query_one('#start_button', Button).disabled = False
            self.query_one('#connect_button', Button).disabled = True
            self.query_one('#disconnect_button', Button).disabled = False
            self.render_status()
        except Exception as exc:
            self._report_exception('connect_output', exc)

    def action_disconnect(self):
        self._clear_log('connect_output')
        try:
            if self.match is not None:
                self.match.stop()
            self.match = None
            mnist_hint.match = None
            self.sub_title = f'Not connected · team {self.config.team_name!r}'
            self.query_one('#connect_button', Button).disabled = False
            self.query_one('#disconnect_button', Button).disabled = True
            self.query_one('#start_button', Button).disabled = True
            self.query_one('#play_mode', RadioSet).disabled = True
            self.query_one('#hint_button', Button).disabled = True
            self.query_one('#pause_button', Button).disabled = True
            self.query_one('#resume_button', Button).disabled = True
            self.query_one('#stop_button', Button).disabled = True
            self.render_status()
            self._log_to('connect_output', 'Disconnected -- edit the connection fields above if needed, then click Connect again.')
        except Exception as exc:
            self._report_exception('connect_output', exc)

    # -- Pre-Match Testing tab actions --------------------------------------
    def action_camera_capture_test(self):
        try:
            frame = detection.camera_capture_test()
            self.notify(f'Captured a {frame.shape[1]}x{frame.shape[0]} frame from {detection.camera_device}. See debug/detection.jpg.')
        except Exception as exc:
            self.notify(f'Capture failed: {exc}', severity='error')

    def action_camera_debug(self):
        self._clear_log('camera_debug_output')
        pos = self.query_one('#camera_debug_position_input', Input).value
        try:
            detection.camera_debug_one_shot(pos)
            self._log_to('camera_debug_output', (
                f'Marker orientation: TL={detection.BORDER_MARKER_TL}, TR={detection.BORDER_MARKER_TR}, '
                f'BR={detection.BORDER_MARKER_BR}, BL={detection.BORDER_MARKER_BL}; '
                f'camera rotation={detection.BOARD_CAMERA_ROTATION_DEGREES:.1f} degrees. '
                'See debug/alignment.jpg and debug/processed_crop.jpg.'
            ))
        except Exception as exc:
            self._report_exception('camera_debug_output', exc)

    def action_mnist_debug(self):
        self._clear_log('mnist_output')
        self._log_to('mnist_output', 'Debug MNIST One-Shot')
        try:
            device = self.query_one('#mnist_camera_select', Select).value
            mnist_hint.capture_mnist_digit(device=device, show=True)
            self._log_to('mnist_output', 'See debug/mnist_digit.jpg.')
        except Exception as exc:
            self._report_exception('mnist_output', exc)

    # -- Match tab actions ---------------------------------------------------
    def action_start(self):
        try:
            referee_id = self.query_one('#referee_input', Input).value.strip()
            if not referee_id:
                raise ValueError('Enter the assigned Referee ID before Start (for example arena-1-referee).')
            self.match.client.referee_id = referee_id
            # match.start() already ran at Connect -- this button now only
            # arms the play controls below, it doesn't start message reception.
            self.query_one('#play_mode', RadioSet).disabled = False
            self.query_one('#hint_button', Button).disabled = False
            self.query_one('#pause_button', Button).disabled = False
            self.query_one('#stop_button', Button).disabled = False
            if self.query_one('#control_auto', RadioButton).value:
                self.query_one('#mode_play', RadioButton).value = True  # triggers on_play_mode_changed
                self.notify('Match client started in Auto mode -- will play automatically as soon as game_start arrives.')
            else:
                self.notify('Match client started in Wait Mode -- waiting for game_start. Switch to Play Mode when ready.')
        except Exception as exc:
            self.notify(str(exc), severity='error')

    def action_pause(self):
        self.match.pause()
        self.query_one('#pause_button', Button).disabled = True
        self.query_one('#resume_button', Button).disabled = False

    def action_resume(self):
        self.match.resume()
        self.query_one('#pause_button', Button).disabled = False
        self.query_one('#resume_button', Button).disabled = True

    def action_stop(self):
        self.match.stop()
        self.query_one('#start_button', Button).disabled = False
        self.query_one('#play_mode', RadioSet).disabled = True
        self.query_one('#hint_button', Button).disabled = True
        self.query_one('#pause_button', Button).disabled = True
        self.query_one('#resume_button', Button).disabled = True
        self.query_one('#stop_button', Button).disabled = True

    def on_radio_set_changed(self, event: RadioSet.Changed) -> None:
        if event.radio_set.id == 'play_mode' and self.match is not None:
            enabled = event.pressed.id == 'mode_play'
            self.match.set_play_mode(enabled)

    # -- Hints tab actions ----------------------------------------------------
    def action_queue_hint(self):
        obj = self.query_one('#hint_object_input', Input).value.strip()
        self._clear_log('hint_output')
        if not obj:
            self._log_to('hint_output', 'Enter an object name first.')
            return
        self.match.queue_hint(obj)
        self._log_to('hint_output', f"Hint request sent: {obj!r}. The referee queues it automatically if this is the opponent's turn.")

    def action_solve_riddle(self):
        if self.match is None or not self.match.pregame_riddle:
            self.notify('No pre-game riddle received yet.')
            return
        try:
            result = hints.solve_pregame_riddle_with_llm(self.match.pregame_riddle)
            self.match.pregame_riddle_answer = result['answer']
            self.notify(f"LLM answer: {result['answer']}")
            self.render_status()
        except Exception as exc:
            self.notify(str(exc), severity='error')

    def action_resolve_free_hint(self):
        if self.match is None or not self.match.free_hint_text:
            self.notify('No free hint text assembled yet.')
            return
        try:
            result = hints.solve_free_hint(self.match.free_hint_text)
            with self.match.lock:
                self.match.free_hint_item = result['item'] or None
                self.match.free_hint_direction = result['direction'] or None
                self.match.free_hint_solve_error = None
            self.notify(f"Free hint decoded: item={result['item']!r}, quadrant={result['direction']!r}")
            self.render_status()
        except Exception as exc:
            self.notify(str(exc), severity='error')

    def action_override_start(self):
        self._clear_log('hint_output')
        self._log_to('hint_output', 'Start Manual Override')
        try:
            card_name = self.query_one('#override_name_input', Input).value.strip()
            if not card_name:
                raise ValueError('Enter a card name before starting a manual override.')
            mnist_hint.paid_hint_state.update(active=True, name=card_name, row=None)
            self.query_one('#override_capture_button', Button).label = 'Capture Row'
            self.query_one('#override_capture_button', Button).disabled = False
            self._log_to('hint_output', f"Manual override started for '{card_name}'. Capture the row digit, then the column digit.")
        except Exception as exc:
            self._report_exception('hint_output', exc)

    def action_override_capture(self):
        self._clear_log('hint_output')
        self._log_to('hint_output', 'Capture Digit')
        try:
            state = mnist_hint.paid_hint_state
            if not state['active']:
                raise RuntimeError('Click Start Manual Override first.')
            if self.query_one('#mnist_source_saved', RadioButton).value:
                digit_choice = self.query_one('#saved_digit_select', Select).value
                digit = mnist_hint.predict_mnist_digit_from_saved_image(digit_choice, show=True)
            else:
                device = self.query_one('#mnist_camera_select', Select).value
                digit = mnist_hint.capture_mnist_digit(device=device, show=True)

            if state['row'] is None:
                state['row'] = digit
                self.query_one('#override_capture_button', Button).label = 'Capture Column'
                self._log_to('hint_output', f'Captured row: {digit}. Now capture the column digit.')
                return

            row, col = state['row'], digit
            result = mnist_hint.inject_paid_hint(state['name'], row, col)
            mnist_hint.paid_hint_state.update(active=False, name='', row=None)
            self.query_one('#override_capture_button', Button).label = 'Capture Row'
            self.query_one('#override_capture_button', Button).disabled = True
            self._log_to('hint_output', str(result))
        except Exception as exc:
            self._report_exception('hint_output', exc)

    def action_overrides_show(self):
        self._clear_log('hint_output')
        self._log_to('hint_output', 'Manual Overrides')
        if not mnist_hint.paid_hints:
            self._log_to('hint_output', 'No manual overrides stored yet.')
        for pos, description in sorted(mnist_hint.paid_hints.items()):
            self._log_to('hint_output', f'  {pos}: {description}')

    def action_overrides_clear(self):
        mnist_hint.paid_hints.clear()
        self._clear_log('hint_output')
        self._log_to('hint_output', 'Manual overrides cleared.')

    # -- button dispatch -----------------------------------------------------
    def on_button_pressed(self, event: Button.Pressed) -> None:
        handlers = {
            'set_time_button': self.action_set_board_time,
            'sync_time_button': self.action_sync_board_time,
            'test_connectivity_button': self.action_test_connectivity,
            'connect_button': self.action_connect,
            'disconnect_button': self.action_disconnect,
            'camera_capture_test_button': self.action_camera_capture_test,
            'camera_debug_button': self.action_camera_debug,
            'mnist_debug_button': self.action_mnist_debug,
            'start_button': self.action_start,
            'pause_button': self.action_pause,
            'resume_button': self.action_resume,
            'stop_button': self.action_stop,
            'hint_button': self.action_queue_hint,
            'solve_riddle_button': self.action_solve_riddle,
            'resolve_free_hint_button': self.action_resolve_free_hint,
            'override_start_button': self.action_override_start,
            'override_capture_button': self.action_override_capture,
            'overrides_show_button': self.action_overrides_show,
            'overrides_clear_button': self.action_overrides_clear,
        }
        handler = handlers.get(event.button.id)
        if handler is not None:
            handler()

    def on_checkbox_changed(self, event: Checkbox.Changed) -> None:
        if event.checkbox.id == 'show_raw_log_checkbox':
            self.render_status()

    # -- status rendering (mirrors render_status() in the notebook) ---------
    def _on_match_update(self):
        """MatchClient calls this from both background threads (the poll
        loop, reveal worker, LLM solvers, LED flashes) and directly from the
        app thread (button handlers like action_queue_hint -> queue_hint()
        -> on_update()). call_from_thread() only works from a genuinely
        different thread than the app, so route accordingly."""
        if threading.current_thread() is self._app_thread:
            self.render_status()
        else:
            self.call_from_thread(self.render_status)

    def render_status(self):
        match = self.match
        stage_display = self.query_one('#stage_display', Static)
        status_display = self.query_one('#status_display', Static)
        board_display = self.query_one('#board_display', Static)

        if match is None:
            stage_display.update('')
            status_display.update('[i]Not connected.[/i]')
            board_display.update('')
            led.set_status_led(None)
            return

        stage_display.update(render_stage_tracker(match.stage))

        lines = []
        if match.last_revealed_pos:
            lines.append(f'[bold black on yellow] FLIP THE PHYSICAL CARD AT {match.last_revealed_pos} NOW [/]')

        scores_text = ', '.join(f'{team}: {score}' for team, score in match.scores.items()) or 'no score yet'
        pairs_text = (
            f'{match.total_pairs - match.pairs_remaining}/{match.total_pairs} pairs'
            if match.pairs_remaining is not None else f'0/{match.total_pairs} pairs'
        )
        lines.append(f'Match #{match.match_number}   Scores: {scores_text}   ({pairs_text})')

        if match.current_turn:
            flip_num = match.current_turn.get('flip_num')
            flip_num_text = f', server flip {flip_num}' if flip_num is not None else ''
            lines.append(f"[bold]Current turn:[/bold] #{match.current_turn['turn_number']} - {match.current_turn['active_team']}{flip_num_text}")
        else:
            lines.append('[bold]Current turn:[/bold] waiting for the server')

        lines.append(f"[bold]Cards selected:[/bold] {', '.join(match.selected_positions) if match.selected_positions else 'none yet'}")
        lines.append(
            f'[bold]Memory:[/bold] {len(match.board_memory)} revealed cells, '
            f'{len(match.reveal_history)} reveal records, {len(match.confirmed_matches)} confirmed matches'
        )
        if match.last_hint:
            lines.append(f'Last hint: {match.last_hint}')
        if match.pending_hint_object:
            lines.append(f'Hint request pending at referee: {match.pending_hint_object}')
        if match.pregame_riddle:
            lines.append(f'Pre-game riddle: {match.pregame_riddle}')
            if match.pregame_riddle_answer:
                lines.append(f'[bold white on green] LLM answer: {match.pregame_riddle_answer} [/]')
        if match.free_hint_item or match.free_hint_direction:
            free_hint_line = f"Free hint decoded: [bold]{match.free_hint_item or '?'}[/bold] in the [bold]{match.free_hint_direction or '?'}[/bold] quadrant"
            if match.free_hint_solve_error:
                free_hint_line += f' [red](retry failed: {match.free_hint_solve_error})[/red]'
            lines.append(free_hint_line)
        elif match.free_hint_solve_error:
            lines.append(f'Free hint: could not auto-decode ({match.free_hint_solve_error}) -- raw text: {match.free_hint_text}')
        elif match.free_hint_text:
            lines.append(f'Free hint (assembled, decoding...): {match.free_hint_text}')
        elif match.free_hint_fragments:
            lines.append(f'Free hint: {len(match.free_hint_fragments)}/{match.free_hint_total} fragments received')
        if match.genesis_team_id:
            connection_state = 'connected' if match.genesis_sim is not None else 'not connected'
            lines.append(f'[white on blue] Genesis: {match.genesis_team_id} ({connection_state})  {match.genesis_url} [/]')

        status_display.update('\n'.join(lines))

        rows = []
        for row in range(detection.GRID_ROWS):
            cells = []
            for col in range(detection.GRID_COLS):
                pos = detection.pos_name(row, col)
                label = match.board_memory.get(pos, ' ')
                if pos in match.matched_positions:
                    style = 'black on green'
                elif pos in match.board_memory:
                    style = 'black on white'
                else:
                    style = 'white on grey15'
                cells.append(f'[{style}] {pos}:{label:<10} [/]')
            rows.append(' '.join(cells))
        board_display.update('\n'.join(rows))

        self.query_one('#start_button', Button).disabled = True
        self.query_one('#play_mode', RadioSet).disabled = match.game_over
        self.query_one('#hint_button', Button).disabled = match.game_over
        self.query_one('#pause_button', Button).disabled = match.game_over
        self.query_one('#resume_button', Button).disabled = match.game_over
        self.query_one('#stop_button', Button).disabled = False

        if self.query_one('#show_raw_log_checkbox', Checkbox).value:
            wire_log = self.query_one('#wire_log', RichLog)
            wire_log.clear()
            for entry in match.log[-40:]:
                arrow = '->' if entry['direction'] == 'send' else '<-'
                wire_log.write(f"{arrow} {json.dumps(entry['message'], ensure_ascii=False)}")

        led.set_status_led(_status_led_color_for_match(match))
