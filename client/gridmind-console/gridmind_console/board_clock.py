"""Board clock -- console port of notebook Section 1a's pure logic
(the Set Board Time / Sync From HTTP buttons are ported into tui.py instead)."""
import subprocess
import urllib.request
import email.utils


def set_board_time(date_string):
    """Sets the board's system clock via `date -s`. Runs as root already
    on these boards, so no sudo is needed."""
    result = subprocess.run(['date', '-s', date_string], capture_output=True, text=True)
    if result.returncode != 0:
        raise RuntimeError(f'Failed to set date: {result.stderr.strip()}')
    now = subprocess.run(['date'], capture_output=True, text=True).stdout.strip()
    print(f'Board time set to: {now}')


def sync_board_time_from_http(url):
    """Best-effort: derive the current time from an HTTP server's Date
    header instead of typing it in by hand -- point this at the broker, the
    master, or any other host reachable from the board."""
    request = urllib.request.Request(url, method='HEAD')
    with urllib.request.urlopen(request, timeout=5) as response:
        date_header = response.headers.get('Date')
    if not date_header:
        raise RuntimeError(f'{url} did not return a Date header.')
    parsed = email.utils.parsedate_to_datetime(date_header)
    set_board_time(parsed.strftime('%Y-%m-%d %H:%M:%S'))
