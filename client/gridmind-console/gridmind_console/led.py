"""Status LED (Grove RGB LED Stick) -- console port of notebook Section 1b.

Optional physical status indicator. Best-effort: if no stick is plugged in,
set_status_led() becomes a silent no-op and the match runs exactly the same.
"""

STATUS_LED_COUNT = 10  # Grove RGB LED Stick has 10 pixels


def rgb(r, g, b):
    return int('0x{:02x}{:02x}{:02x}'.format(r, g, b), 16)


STATUS_LED_COLORS = {
    'connecting': rgb(0, 0, 255),      # blue -- pregame (join/riddle/free hint)
    'your_turn': rgb(0, 255, 0),       # green -- your flip/detect in progress
    'opponent_turn': rgb(60, 30, 0),   # dim amber -- opponent's turn, just watching
    'hint_pending': rgb(255, 165, 0),  # orange -- a queued hint is resolving
    'match_found': rgb(0, 255, 255),   # cyan flash -- a pair was just matched
    'no_match': rgb(255, 0, 0),        # red flash -- a guess just missed
    'won': rgb(0, 255, 0),
    'lost': rgb(255, 0, 0),
    'tied': rgb(150, 0, 150),
}

status_led = None


def init_status_led(overlay, log=print):
    """Best-effort -- must never raise; a missing stick just means
    set_status_led() below becomes a no-op for the rest of the session."""
    global status_led
    try:
        from pynq_peripherals import PmodGroveAdapter
        adapter = PmodGroveAdapter(overlay.PMODA, G4='grove_led_stick')
        status_led = adapter.G4
        status_led.clear()
        log('Status LED ready on PMODA/G4.')
    except Exception as exc:
        status_led = None
        log(f'No Grove RGB LED Stick detected on PMODA/G4 -- continuing without the status LED ({exc}).')
    return status_led


def set_status_led(color, lit_count=STATUS_LED_COUNT):
    """Best-effort -- a missing/disconnected LED stick must never interrupt a match."""
    if status_led is None:
        return
    try:
        status_led.clear()
        if color is not None:
            for pixel in range(lit_count):
                status_led.set_pixel(pixel, color)
            status_led.show()
    except Exception as exc:
        print(f'[status-led] update failed: {exc}')
