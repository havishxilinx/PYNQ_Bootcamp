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

This needs to run as **root** with the board's own PYNQ/DPU Python
environment, not a fresh venv built from a plain `python3`. On most
Kria/KV260 images, Jupyter itself runs as root under `/home/root`, and the
actual `pynq`/`pynq_dpu`/`pynqp2p`/`cv2` packages live in a *dedicated*
venv (commonly `/usr/local/share/pynq-venv/`), separate from the system
`python3`. Building a fresh `--system-site-packages` venv from bare
`python3` inherits from the wrong base interpreter and silently misses
all of those packages -- use the board's own PYNQ venv directly instead.

```bash
sudo -i                             # a real root shell/environment, not a one-off `sudo <cmd>`
cd /home/root/jupyter_notebooks/.../gridmind-console   # wherever it's deployed

# find the board's actual PYNQ venv if you don't already know it -- open a
# working notebook (e.g. PYNQ 301 - Object Detection) and run this in a cell:
#   import sys; print(sys.executable)
# it'll print something like /usr/local/share/pynq-venv/bin/python3

/usr/local/share/pynq-venv/bin/python3 -m pip install textual
cp config.example.json config.json
# edit config.json: server, broker_key, referee_id, team_name, team_secret, detection_approach, ...

/usr/local/share/pynq-venv/bin/python3 -m gridmind_console.main
```

Run from the same directory as your model files (`tf_yolov3_voc.xmodel`,
`img/voc_classes.txt`, `dpu_mnist_classifier.xmodel`), same as the notebook.

### Troubleshooting

- **`PermissionError` extracting the vendored `pynqp2p` wheel, or
  `ModuleNotFoundError: No module named 'pynqp2p'`** -- you're not root.
  `main.py` bootstraps `pynqp2p` from a hardcoded `/home/root/...` path;
  a non-root user can't write there. Use `sudo -i` (a full root shell),
  not a one-off `sudo <single command>` -- the latter silently swaps out
  which Python/venv gets used partway through and reintroduces this same
  class of error for `pynq_dpu` instead.
- **`ModuleNotFoundError: No module named 'pynq_dpu'` even as root** --
  your venv was built from the wrong base Python. Find the board's real
  PYNQ venv (see the `sys.executable` trick above) and use *that*
  interpreter directly; don't build a nested `--system-site-packages`
  venv from it either -- that flag doesn't reliably inherit an
  intermediate venv's own installed packages, only its own base's.
- **`xclbinutil` segfaults (`Segmentation fault (core dumped)`), surfacing
  as `FileNotFoundError` on a `t.xclbin` temp file deep in
  `pynq/pl_server/embedded_device.py`** -- this is a broken XRT
  (Xilinx Runtime) install on the board, unrelated to this project
  entirely. Confirm by trying the same `DpuOverlay(...)` cell in a
  working notebook; if it also fails, a reboot is worth trying first,
  otherwise this needs whoever manages the board image.

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
