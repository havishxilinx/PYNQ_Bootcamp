# GridMind Console

A plain-Python, no-Jupyter port of the `PYNQ_302-aruco_border-Red-v2.ipynb`
GridMind competition client. Every cell's logic is ported verbatim (bug
fixes, LED, LLM hint solvers, and the widget dashboard's actions all
included) -- only the UI layer changed, from ipywidgets/browser to a
[Textual](https://textual.textualize.io/) TUI running directly in the
terminal, no kernel or browser involved.

Built specifically to test whether the notebook's crashes ("kernel dies
after a few turns") are a Jupyter-side issue (ipywidgets comm buffering,
`Out[]` history retention) rather than the detection/match logic itself --
none of that infrastructure exists here.

## Setup (on the board)

```bash
cd gridmind-console
python3 -m venv --system-site-packages .venv   # sees the board's pynq_dpu/pynq_peripherals/pynqp2p/cv2
source .venv/bin/activate
uv pip install textual                          # or: pip install textual, if uv isn't set up yet
cp config.example.json config.json
# edit config.json: server, broker_key, referee_id, team_name, team_secret, detection_approach, ...
```

Run from the same directory as your model files (`tf_yolov3_voc.xmodel`,
`img/voc_classes.txt`, `dpu_mnist_classifier.xmodel`), same as the notebook:

```bash
python3 -m gridmind_console.main
```

## What's different from the notebook

- **Config file instead of a Python cell.** Edit `config.json`, not Section 2.
- **Debug images write to `debug/*.jpg`** instead of live inline previews
  (a terminal can't render bitmaps) -- open them in an image viewer
  alongside the TUI if you want to see alignment.jpg / detection.jpg /
  processed_crop.jpg / mnist_digit.jpg update as you use the debug tools.
- **Five tabs instead of one long scroll**: Connect, Pre-Match Testing,
  Match, Hints, Log -- same controls as the notebook dashboard, just
  organized so each screen fits a terminal instead of scrolling forever.
- Everything else -- MatchClient, detection/ArUco/YOLO pipeline, the LED
  status indicator, the LLM riddle/free-hint solvers, Genesis integration,
  the paid-hint MNIST auto-decode, manual override -- is the exact same
  code, unchanged, just organized into modules instead of notebook cells.

## Module map

| Module | Notebook section(s) |
|---|---|
| `config.py` | Section 2 (Match Configuration) |
| `board_clock.py` | Section 1a (pure logic; TUI has the buttons) |
| `led.py` | Section 1b (Status LED) |
| `yolo.py` | Section 3 (YOLO Detection Helpers) + DPU `run()` |
| `detection.py` | Sections 4, 4a, 5, 6 (grid mapping, ArUco dispatch, camera, per-position detection) |
| `hints.py` | Sections 7a, 7b (pre-game riddle + free-hint LLM solvers) |
| `referee_client.py` | Section 8 (Referee Connection) |
| `match_client.py` | Section 9 (Match Client) |
| `mnist_hint.py` | Section 11 (MNIST classifier + manual override) |
| `tui.py` | Section 10 (Widget GUI) + Section 12 (MNIST/override wiring) |
| `main.py` | Entry point -- wires everything, launches the TUI |

## Known limitations vs. the notebook

- No live inline camera/detection preview (see debug/*.jpg workaround above).
- `test_free_hint_solvers()`'s self-test helper wasn't ported (nothing calls
  it automatically in the notebook either -- it's a manual debug helper you
  can still write as a one-off script against `hints.py` if you want it).
