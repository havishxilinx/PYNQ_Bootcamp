"""MNIST digit classifier + manual hint-override helpers -- console port of
notebook Section 11.
"""
import time
from pathlib import Path

import numpy as np
import cv2

from . import detection
from . import yolo

MNIST_MODEL_PATHS = []
MNIST_DIGITS_DIR_CANDIDATES = []

mnist_runner = None
mnist_input_data = None
mnist_output_data = None
mnist_output_size = None
mnist_camera = None
mnist_camera_device = None
paid_hints = {}       # pos -> card name, e.g. {'B3': 'dog'}
paid_hint_state = {'active': False, 'name': '', 'row': None}

mnist_image_widget = None

_overlay = None

# Set by main.py once a MatchClient is connected, so inject_paid_hint() can
# feed the live match -- mirrors the notebook's `match` global.
match = None


def init(overlay, notebook_dir, debug_dir='debug'):
    global _overlay, MNIST_MODEL_PATHS, MNIST_DIGITS_DIR_CANDIDATES, mnist_image_widget
    _overlay = overlay
    notebook_dir = Path(notebook_dir)
    MNIST_MODEL_PATHS = [
        notebook_dir / 'dpu_mnist_classifier.xmodel',
        Path('/home/root/jupyter_notebooks/PYNQ_Bootcamp/bootcamp_sessions/PYNQ 201 - MNIST/dpu_mnist_classifier.xmodel'),
        Path('/home/root/jupyter_notebooks/pynq-dpu/dpu_mnist_classifier.xmodel'),
    ]
    MNIST_DIGITS_DIR_CANDIDATES = [
        notebook_dir / 'mnist_digits_0-9',
        Path('/home/root/jupyter_notebooks/PYNQ_Bootcamp/bootcamp_sessions/mnist_digits_0-9'),
    ]
    mnist_image_widget = detection.FileImageWidget(f'{debug_dir}/mnist_digit.jpg')


def mnist_model_path():
    for path in MNIST_MODEL_PATHS:
        if path.exists():
            return path
    raise FileNotFoundError('Could not find dpu_mnist_classifier.xmodel. Check the PYNQ 201 - MNIST notebook folder.')


def mnist_digits_dir():
    for path in MNIST_DIGITS_DIR_CANDIDATES:
        if path.exists():
            return path
    raise FileNotFoundError(
        'Could not find mnist_digits_0-9 folder. Copy it into bootcamp_sessions or this notebook folder.'
    )


def calculate_mnist_softmax(data):
    result = np.exp(data)
    return result / np.sum(result)


def ensure_mnist_runner():
    global mnist_runner, mnist_input_data, mnist_output_data, mnist_output_size
    if mnist_runner is not None:
        return mnist_runner

    _overlay.load_model(str(mnist_model_path()))
    mnist_runner = _overlay.runner
    input_tensors = mnist_runner.get_input_tensors()
    output_tensors = mnist_runner.get_output_tensors()
    shape_in = tuple(input_tensors[0].dims)
    shape_out = tuple(output_tensors[0].dims)
    mnist_output_size = int(output_tensors[0].get_data_size() / shape_in[0])
    mnist_input_data = [np.empty(shape_in, dtype=np.float32, order='C')]
    mnist_output_data = [np.empty(shape_out, dtype=np.float32, order='C')]
    return mnist_runner


def ensure_yolo_runner():
    """Reloads the YOLO model after an MNIST capture swapped the DPU over --
    call this before any further board detection."""
    global mnist_runner
    yolo.reload()
    mnist_runner = None
    return yolo.dpu


def preprocess_mnist_frame(frame):
    gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
    blurred = cv2.GaussianBlur(gray, (5, 5), 0)
    resized = cv2.resize(blurred, (28, 28), interpolation=cv2.INTER_AREA)

    # MNIST training data is white digit on black background. Invert bright paper backgrounds.
    if np.mean(resized) > 127:
        resized = 255 - resized

    normalized = np.asarray(resized / 255.0, dtype=np.float32)
    return np.expand_dims(normalized, axis=2)


def open_mnist_camera(device=None):
    global mnist_camera, mnist_camera_device
    if mnist_camera is not None and mnist_camera.isOpened():
        mnist_camera.release()

    if device is None:
        device = mnist_camera_device
    if device is None:
        candidates = [candidate for candidate in detection.video_devices() if candidate != detection.camera_device]
        device = candidates[0] if candidates else detection.camera_device
    if device is None:
        raise RuntimeError('No MNIST camera device selected.')

    mnist_camera = detection.open_camera(device)
    mnist_camera_device = device
    return mnist_camera


def read_mnist_camera_frame(device=None, read_attempts=8):
    if device is not None or mnist_camera is None or not mnist_camera.isOpened():
        open_mnist_camera(device)

    for _ in range(max(1, int(read_attempts))):
        ret, frame = mnist_camera.read()
        if ret and frame is not None:
            return frame
        mnist_camera.grab()
        ret, frame = mnist_camera.retrieve()
        if ret and frame is not None:
            return frame
        time.sleep(0.05)

    open_mnist_camera(device)
    ret, frame = mnist_camera.read()
    if ret and frame is not None:
        return frame
    raise RuntimeError('Could not read a frame from the MNIST camera.')


def run_mnist_model_from_processed(digit_image):
    runner = ensure_mnist_runner()
    mnist_input_data[0][0, ...] = digit_image.reshape(mnist_input_data[0].shape[1:])
    job_id = runner.execute_async(mnist_input_data, mnist_output_data)
    runner.wait(job_id)
    logits = mnist_output_data[0].reshape(1, mnist_output_size)[0]
    probabilities = calculate_mnist_softmax(logits)
    prediction = int(probabilities.argmax())
    return prediction, probabilities


def _show_mnist_prediction(frame, digit_image, prediction, confidence):
    annotated = detection.draw_label(frame.copy(), f'MNIST prediction: {prediction} ({confidence:.2f})', frame.shape[1] // 2, 30, (0, 255, 0))
    detection._set_image_widget(mnist_image_widget, annotated)


def predict_mnist_digit_from_frame(frame, show=True):
    digit_image = preprocess_mnist_frame(frame)
    prediction, probabilities = run_mnist_model_from_processed(digit_image)
    confidence = float(probabilities[prediction])

    if show:
        _show_mnist_prediction(frame, digit_image, prediction, confidence)

    # Restore YOLO so normal board detection still works after this capture.
    ensure_yolo_runner()
    print(f'MNIST prediction: {prediction} (confidence={confidence:.3f})')
    return prediction


def capture_mnist_digit(device=None, show=True):
    frame = read_mnist_camera_frame(device=device)
    return predict_mnist_digit_from_frame(frame, show=show)


def decode_digit_png_base64(png_base64, show=True):
    """Decodes a base64 PNG digit image (as delivered by hint_response)
    into a predicted digit 0-9 via the on-board MNIST classifier."""
    import base64
    png_bytes = base64.b64decode(png_base64)
    frame = cv2.imdecode(np.frombuffer(png_bytes, dtype=np.uint8), cv2.IMREAD_COLOR)
    if frame is None:
        raise ValueError('Could not decode the hint digit image.')
    return predict_mnist_digit_from_frame(frame, show=show)


def predict_mnist_digit_from_saved_image(digit_choice, show=True):
    """Debug-only: classify a pre-saved sample digit image from
    mnist_digits_0-9/ instead of the live MNIST camera."""
    image_path = mnist_digits_dir() / f'digit_{int(digit_choice)}.png'
    frame = cv2.imread(str(image_path))
    if frame is None:
        raise FileNotFoundError(f'Could not read saved digit image: {image_path}')
    return predict_mnist_digit_from_frame(frame, show=show)


def inject_paid_hint(card_name, row, col):
    card_name = str(card_name).strip()
    row = int(row)
    col = int(col)
    if not card_name:
        raise ValueError('Enter a card name before using the hint override.')
    if not (0 <= row < detection.GRID_ROWS and 0 <= col < detection.GRID_COLS):
        raise ValueError(f'Hint digits row={row} col={col} are outside the {detection.GRID_ROWS}x{detection.GRID_COLS} grid.')

    pos = detection.pos_name(row, col)
    paid_hints[pos] = card_name
    # Feed the live match the same way the automatic hint_response decode
    # does, so a manual override behaves identically to a successful
    # auto-decode instead of only updating a side dict nothing else reads.
    if match is not None:
        with match.lock:
            match.board_memory[pos] = card_name
            match.last_hint_position = pos
            match.last_hint = f'Manual override: {card_name} at {pos}'
        match.on_update()
    print(f'Hint override stored: {card_name} at {pos}')
    return {'pos': pos, 'description': card_name}
