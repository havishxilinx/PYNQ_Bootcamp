"""Entry point -- console equivalent of running the notebook's Setup/Board
Clock/Status LED/Match Configuration/YOLO/Camera cells in order, then
launching the TUI instead of display()-ing the widget dashboard.

Usage:
    python3 -m gridmind_console.main [--config config.json]
"""
import argparse
import sys
import zipfile
from pathlib import Path


def bootstrap_pynqp2p():
    """Same vendored-wheel extraction the notebook's Setup cell did."""
    wheel = '/home/root/jupyter_notebooks/pynqp2p_pkg/vendor_wheels/getmac-0.9.5-py2.py3-none-any.whl'
    extract_dir = '/home/root/jupyter_notebooks/pynqp2p_pkg/vendor_wheels/extracted'
    if Path(wheel).exists():
        zipfile.ZipFile(wheel).extractall(extract_dir)
        sys.path.insert(0, extract_dir)
        sys.path.insert(0, '/home/root/jupyter_notebooks/pynqp2p_pkg')


def main():
    parser = argparse.ArgumentParser(description='GridMind console client')
    parser.add_argument('--config', default='config.json', help='Path to config.json (see config.example.json)')
    parser.add_argument('--debug-dir', default='debug', help='Directory for debug preview JPEGs')
    args = parser.parse_args()

    from .config import Config
    config = Config.load(args.config)

    bootstrap_pynqp2p()
    import pynqp2p  # noqa: F401 -- confirms the vendored wheel import works before anything else

    ai_helper_dir = '/home/root/jupyter_notebooks/PYNQ_Bootcamp/bootcamp_sessions/ai_llm'
    if ai_helper_dir not in sys.path:
        sys.path.append(ai_helper_dir)

    from pynq_dpu import DpuOverlay

    notebook_301_dir = Path('/home/root/jupyter_notebooks/PYNQ_Bootcamp/bootcamp_sessions/PYNQ 301 - Object Detection')
    notebook_dir = Path(config.notebook_dir).resolve()
    model_path = notebook_dir / config.model_path
    classes_path = notebook_dir / config.classes_path
    if not model_path.exists() and notebook_301_dir.exists():
        notebook_dir = notebook_301_dir
        model_path = notebook_dir / config.model_path
        classes_path = notebook_dir / config.classes_path
    print(f'Using notebook assets from: {notebook_dir}')

    overlay = DpuOverlay('dpu.bit')

    from . import yolo
    yolo.init(overlay, model_path, classes_path)

    from . import led
    led.init_status_led(overlay)

    from . import detection
    detection.init(config, debug_dir=args.debug_dir)

    from . import mnist_hint
    mnist_hint.init(overlay, notebook_dir, debug_dir=args.debug_dir)

    from .tui import GridMindApp
    app = GridMindApp(config)
    app.run()

    if detection.cap is not None:
        detection.cap.release()


if __name__ == '__main__':
    main()
