"""Grid mapping, ArUco/multi-approach dispatch, camera, and per-position
detection -- console port of notebook Sections 4, 4a, 5, and 6. Merged into
one module since they all share the same mutable module-level config state
(GRID_ROWS, GRID_COLS, BOARD_CORNERS, DETECTION_MODE, CARD_POSITION_MODE,
DETECTION_APPROACH, GRID_CROP_SIZE) exactly as they did as notebook globals.

Debug preview images (alignment/detection/processed-crop/MNIST-digit) are
written to disk as JPEGs instead of ipywidgets.Image, via FileImageWidget --
a drop-in replacement for `.value = jpeg_bytes` so every function that
already does that (show_detection_frame, show_processed_grid_crop, etc.)
needed zero changes.
"""
import os
import time

import numpy as np
import cv2

from . import yolo

# -- set by init(), mirrors the notebook's Section 2 config cell -----------
GRID_ROWS = 5
GRID_COLS = 6
BOARD_CORNERS = None
DETECTION_APPROACH = 'aruco_border'
DETECTION_MODE = 'grid'
CARD_POSITION_MODE = 'yolo_center'

ARUCO_DICTIONARY_NAME = 'DICT_4X4_50'
BORDER_MARKER_TL = 30
BORDER_MARKER_TR = 31
BORDER_MARKER_BR = 33
BORDER_MARKER_BL = 32
BOARD_MARKER_CENTERS = {}
BOARD_CAMERA_ROTATION_DEGREES = None

GRID_CROP_SIZE = (416, 416)
YOLO_FALLBACK_SCORE_THRESHOLDS = [0.15, 0.1, 0.05]
DETECT_PHOTO_COUNT = 2
DETECT_SETTLE_SECONDS = 0.3
DETECT_FLUSH_FRAMES = 8

MAX_OBJECTS = 2
TURN_DEBUG_SHOW_LABELS = True
TURN_DEBUG_SHOW_ALL_DETECTIONS = False

_APPROACH_SETTINGS = {
    'yolo_full_frame': ('full_frame', 'yolo_center'),
    'yolo_grid_crops': ('grid', 'yolo_center'),
    'aruco_border': ('grid', 'yolo_center'),
    'aruco_per_card': ('full_frame', 'aruco_id'),
    'aruco_per_card_grid_crops': ('grid', 'aruco_id'),
    'aruco_per_card_crop': ('full_frame', 'yolo_center'),
    'aruco_per_card_crop_verified': ('full_frame', 'yolo_center'),
}

CAMERA_SIZE = (1920, 1080)
CAMERA_FPS = 15
CAMERA_BUFFER_SIZE = 1

cap = None
camera_device = None


class FileImageWidget:
    """Drop-in replacement for ipywidgets.Image's .value setter -- writes
    the JPEG bytes to disk instead of pushing them to a browser frontend.
    Every function below that does `widget.value = jpeg_bytes` needed zero
    changes to work against this."""

    def __init__(self, path):
        self.path = path

    @property
    def value(self):
        return None

    @value.setter
    def value(self, jpeg_bytes):
        tmp_path = f'{self.path}.tmp'
        with open(tmp_path, 'wb') as f:
            f.write(jpeg_bytes)
        os.replace(tmp_path, self.path)


alignment_image_widget = None
detection_image_widget = None
processed_crop_image_widget = None


def init(config, debug_dir='debug'):
    """Console equivalent of running Section 2 + the top of Section 4a +
    Section 5's cell in the notebook."""
    global GRID_ROWS, GRID_COLS, BOARD_CORNERS, DETECTION_APPROACH
    global BORDER_MARKER_TL, BORDER_MARKER_TR, BORDER_MARKER_BR, BORDER_MARKER_BL
    global alignment_image_widget, detection_image_widget, processed_crop_image_widget
    global cap, camera_device

    GRID_ROWS = config.grid_rows
    GRID_COLS = config.grid_cols
    BOARD_CORNERS = np.float32(config.board_corners) if config.board_corners else None
    DETECTION_APPROACH = config.detection_approach
    BORDER_MARKER_TL = config.border_marker_tl
    BORDER_MARKER_TR = config.border_marker_tr
    BORDER_MARKER_BR = config.border_marker_br
    BORDER_MARKER_BL = config.border_marker_bl

    os.makedirs(debug_dir, exist_ok=True)
    alignment_image_widget = FileImageWidget(os.path.join(debug_dir, 'alignment.jpg'))
    detection_image_widget = FileImageWidget(os.path.join(debug_dir, 'detection.jpg'))
    processed_crop_image_widget = FileImageWidget(os.path.join(debug_dir, 'processed_crop.jpg'))

    set_detection_approach(DETECTION_APPROACH)

    if cap is not None:
        cap.release()
    try:
        cap, camera_device = scan_working_camera()
    except RuntimeError as exc:
        print(f'{exc} Continuing without a camera.')
        cap, camera_device = None, None


def set_detection_mode(mode):
    global DETECTION_MODE
    if mode not in ('grid', 'full_frame'):
        raise ValueError("mode must be 'grid' or 'full_frame'")
    DETECTION_MODE = mode
    return DETECTION_MODE


def set_card_position_mode(mode):
    global CARD_POSITION_MODE
    if mode not in ('yolo_center', 'aruco_id'):
        raise ValueError("mode must be 'yolo_center' or 'aruco_id'")
    CARD_POSITION_MODE = mode
    return CARD_POSITION_MODE


def set_detection_approach(approach):
    global DETECTION_APPROACH
    if approach not in _APPROACH_SETTINGS:
        raise ValueError(f"Unknown approach: {approach!r}. Choose from: {list(_APPROACH_SETTINGS)}")
    DETECTION_APPROACH = approach
    det_mode, pos_mode = _APPROACH_SETTINGS[approach]
    set_detection_mode(det_mode)
    set_card_position_mode(pos_mode)
    print(f'[detection] approach set to: {approach}')


# -- Section 4: grid mapping -------------------------------------------------

def pos_name(row, col):
    return f'{chr(ord("A") + row)}{col + 1}'


def parse_pos(pos):
    row = ord(pos[0].upper()) - ord('A')
    col = int(pos[1:]) - 1
    return row, col


def _quadrant_for_position(pos):
    """Mirrors the referee's own quadrant_for_position (gridmind-referee's
    hints.rs) so the free hint's decoded quadrant lines up with the same
    grid split it was generated from -- ceil(rows/2)/ceil(cols/2) midpoint,
    1-indexed, exactly as there."""
    row, col = parse_pos(pos)
    row_num, col_num = row + 1, col + 1
    top = row_num <= (GRID_ROWS + 1) // 2
    left = col_num <= (GRID_COLS + 1) // 2
    return f"{'top' if top else 'bottom'} {'left' if left else 'right'}"


def board_source_corners(frame_shape):
    height, width = frame_shape[:2]
    if BOARD_CORNERS is not None:
        return np.float32(BOARD_CORNERS)
    return np.float32([[0, 0], [width - 1, 0], [width - 1, height - 1], [0, height - 1]])


def board_grid_corners():
    """Canonical logical orientation: A1 is always the top-left cell."""
    return np.float32([
        [0, 0], [GRID_COLS, 0], [GRID_COLS, GRID_ROWS], [0, GRID_ROWS],
    ])


def board_transform(frame_shape):
    return cv2.getPerspectiveTransform(board_source_corners(frame_shape), board_grid_corners())


def frame_transform(frame_shape):
    return cv2.getPerspectiveTransform(board_grid_corners(), board_source_corners(frame_shape))


def warp_board_to_canonical(frame, cell_pixels=120):
    """Perspective-warp the full board so semantic TL marker is image TL."""
    width = int(GRID_COLS * cell_pixels)
    height = int(GRID_ROWS * cell_pixels)
    destination = np.float32([
        [0, 0], [width - 1, 0], [width - 1, height - 1], [0, height - 1],
    ])
    transform = cv2.getPerspectiveTransform(board_source_corners(frame.shape), destination)
    return cv2.warpPerspective(frame, transform, (width, height))


def draw_canonical_board(frame, selected_pos=None, cell_pixels=120):
    """Return an oriented board preview with A1 visibly in the top-left."""
    board = warp_board_to_canonical(frame, cell_pixels=cell_pixels)
    height, width = board.shape[:2]
    for col in range(GRID_COLS + 1):
        x = int(round(col * width / GRID_COLS))
        cv2.line(board, (min(x, width - 1), 0), (min(x, width - 1), height - 1), (0, 255, 0), 2)
    for row in range(GRID_ROWS + 1):
        y = int(round(row * height / GRID_ROWS))
        cv2.line(board, (0, min(y, height - 1)), (width - 1, min(y, height - 1)), (0, 255, 0), 2)
    for row in range(GRID_ROWS):
        for col in range(GRID_COLS):
            x = int(col * width / GRID_COLS) + 8
            y = int(row * height / GRID_ROWS) + 24
            cv2.putText(board, pos_name(row, col), (x, y), cv2.FONT_HERSHEY_SIMPLEX, 0.55, (0, 255, 0), 2, cv2.LINE_AA)
    if selected_pos:
        row, col = parse_pos(selected_pos)
        x1 = int(col * width / GRID_COLS)
        y1 = int(row * height / GRID_ROWS)
        x2 = int((col + 1) * width / GRID_COLS) - 1
        y2 = int((row + 1) * height / GRID_ROWS) - 1
        cv2.rectangle(board, (x1, y1), (x2, y2), (0, 255, 255), 5)
    rotation_text = f' | camera rotation {BOARD_CAMERA_ROTATION_DEGREES:.1f} deg' if BOARD_CAMERA_ROTATION_DEGREES is not None else ''
    cv2.putText(board, f'Auto-oriented: A1 top-left{rotation_text}', (10, height - 12), cv2.FONT_HERSHEY_SIMPLEX, 0.55, (0, 255, 255), 2, cv2.LINE_AA)
    return board


def grid_cell_quad(row, col, frame_shape):
    to_frame = frame_transform(frame_shape)
    grid_points = np.float32([[[col, row], [col + 1, row], [col + 1, row + 1], [col, row + 1]]])
    return cv2.perspectiveTransform(grid_points, to_frame).reshape(4, 2).astype(np.float32)


def crop_grid_cell(frame, row, col):
    """Returns (crop, src_quad, dst_quad) -- the quads let a caller map a
    detection made in crop-space back to real frame coordinates."""
    width, height = GRID_CROP_SIZE
    src = grid_cell_quad(row, col, frame.shape)
    dst = np.float32([[0, 0], [width - 1, 0], [width - 1, height - 1], [0, height - 1]])
    transform = cv2.getPerspectiveTransform(src, dst)
    crop = cv2.warpPerspective(frame, transform, (width, height))
    return crop, src, dst


def box_center(box):
    y_min, x_min, y_max, x_max = map(float, box)
    return (x_min + x_max) / 2.0, (y_min + y_max) / 2.0


def grid_position_for_point(point, frame_shape):
    point_arr = np.float32([[[float(point[0]), float(point[1])]]])
    grid_x, grid_y = cv2.perspectiveTransform(point_arr, board_transform(frame_shape))[0][0]
    col = int(np.clip(np.floor(grid_x), 0, GRID_COLS - 1))
    row = int(np.clip(np.floor(grid_y), 0, GRID_ROWS - 1))
    return {
        'row': row,
        'col': col,
        'cell': pos_name(row, col),
        'center': (float(point[0]), float(point[1])),
    }


def grid_position_for_aruco_id(marker_id, fallback_center):
    marker_id = int(marker_id)
    cell_index = marker_id % (GRID_ROWS * GRID_COLS)
    row = cell_index // GRID_COLS
    col = cell_index % GRID_COLS
    return {
        'row': row,
        'col': col,
        'cell': pos_name(row, col),
        'center': tuple(map(float, fallback_center)),
    }


def aruco_grid_position(marker_id, center, frame_shape):
    if CARD_POSITION_MODE == 'aruco_id':
        return grid_position_for_aruco_id(marker_id, center)
    return grid_position_for_point(center, frame_shape)


def crop_box_to_frame_box(crop_box, src_quad, dst_quad, frame_shape):
    y_min, x_min, y_max, x_max = map(float, crop_box)
    crop_corners = np.float32([[
        [x_min, y_min], [x_max, y_min], [x_max, y_max], [x_min, y_max],
    ]])
    inverse_transform = cv2.getPerspectiveTransform(dst_quad, src_quad)
    frame_corners = cv2.perspectiveTransform(crop_corners, inverse_transform).reshape(4, 2)
    frame_height, frame_width = frame_shape[:2]
    xs = np.clip(frame_corners[:, 0], 0, frame_width - 1)
    ys = np.clip(frame_corners[:, 1], 0, frame_height - 1)
    return [float(np.min(ys)), float(np.min(xs)), float(np.max(ys)), float(np.max(xs))]


def crop_box_center_to_frame(crop_box, src_quad, dst_quad):
    y_min, x_min, y_max, x_max = map(float, crop_box)
    crop_center = np.float32([[[((x_min + x_max) / 2.0), ((y_min + y_max) / 2.0)]]])
    inverse_transform = cv2.getPerspectiveTransform(dst_quad, src_quad)
    center = cv2.perspectiveTransform(crop_center, inverse_transform)[0][0]
    return (float(center[0]), float(center[1]))


def crop_points_to_frame(points, src_quad, dst_quad):
    inverse_transform = cv2.getPerspectiveTransform(dst_quad, src_quad)
    frame_points = cv2.perspectiveTransform(np.float32([points]), inverse_transform)[0]
    return frame_points.astype(np.float32)


def draw_grid(frame):
    frame = frame.copy()
    to_frame = frame_transform(frame.shape)
    for col in range(GRID_COLS + 1):
        p1, p2 = cv2.perspectiveTransform(np.float32([[[col, 0]], [[col, GRID_ROWS]]]), to_frame).reshape(2, 2).astype(int)
        cv2.line(frame, tuple(p1), tuple(p2), (0, 255, 0), 1)
    for row in range(GRID_ROWS + 1):
        p1, p2 = cv2.perspectiveTransform(np.float32([[[0, row]], [[GRID_COLS, row]]]), to_frame).reshape(2, 2).astype(int)
        cv2.line(frame, tuple(p1), tuple(p2), (0, 255, 0), 1)
    return frame


def draw_label(frame, label, x, y, color=(255, 255, 255)):
    size = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.6, 2)[0]
    y = max(y, size[1] + 8)
    cv2.rectangle(frame, (x - size[0] // 2 - 4, y - size[1] - 8), (x + size[0] // 2 + 4, y + 4), (0, 0, 0), -1)
    return cv2.putText(frame, label, (x - size[0] // 2, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, color, 2, cv2.LINE_AA)


# -- Section 4a: ArUco markers & multi-approach detection dispatch ---------

def aruco_dictionary():
    if not hasattr(cv2, 'aruco'):
        raise RuntimeError('OpenCV ArUco support is not available. Install opencv-contrib-python or use an OpenCV build with cv2.aruco.')
    dictionary_id = getattr(cv2.aruco, ARUCO_DICTIONARY_NAME, None)
    if dictionary_id is None:
        raise ValueError(f'Unknown ArUco dictionary: {ARUCO_DICTIONARY_NAME}')
    return cv2.aruco.getPredefinedDictionary(dictionary_id)


def aruco_detector_parameters():
    if hasattr(cv2.aruco, 'DetectorParameters'):
        return cv2.aruco.DetectorParameters()
    return cv2.aruco.DetectorParameters_create()


def detect_aruco_corners_and_ids(frame):
    gray = cv2.cvtColor(frame, cv2.COLOR_BGR2GRAY)
    dictionary = aruco_dictionary()
    parameters = aruco_detector_parameters()
    if hasattr(cv2.aruco, 'ArucoDetector'):
        detector = cv2.aruco.ArucoDetector(dictionary, parameters)
        corners, ids, rejected = detector.detectMarkers(gray)
    else:
        corners, ids, rejected = cv2.aruco.detectMarkers(gray, dictionary, parameters=parameters)
    if ids is None:
        return [], np.array([], dtype=np.int32)
    return corners, ids.flatten().astype(np.int32)


def calibrate_from_border_markers(frame):
    """Update the board homography from semantic TL/TR/BR/BL marker IDs."""
    global BOARD_CORNERS, BOARD_MARKER_CENTERS, BOARD_CAMERA_ROTATION_DEGREES
    corners, ids = detect_aruco_corners_and_ids(frame)
    marker_roles = {
        BORDER_MARKER_TL: ('TL', 0),
        BORDER_MARKER_TR: ('TR', 1),
        BORDER_MARKER_BR: ('BR', 2),
        BORDER_MARKER_BL: ('BL', 3),
    }
    found = {}
    centers_by_role = {}
    for marker_corners, marker_id in zip(corners, ids):
        role_info = marker_roles.get(int(marker_id))
        if role_info is None:
            continue
        role, corner_index = role_info
        center = np.mean(marker_corners.reshape(4, 2), axis=0).astype(np.float32)
        found[corner_index] = center
        centers_by_role[role] = center
    if len(found) == 4:
        candidate = np.float32([found[0], found[1], found[2], found[3]])
        if abs(float(cv2.contourArea(candidate))) < 100.0:
            raise RuntimeError('Border marker centers form a degenerate board quadrilateral.')
        BOARD_CORNERS = candidate
        BOARD_MARKER_CENTERS = centers_by_role
        top_edge = found[1] - found[0]
        BOARD_CAMERA_ROTATION_DEGREES = float(np.degrees(np.arctan2(top_edge[1], top_edge[0])))
    return len(found)


def aruco_records_for_frame(frame, detection_mode):
    """Detect ArUco markers on the full frame, mapping each to a grid cell.
    Only card-range IDs (0..GRID_ROWS*GRID_COLS-1) are returned; configured
    border marker IDs never appear as false card detections."""
    corners, ids = detect_aruco_corners_and_ids(frame)
    records = []
    card_id_limit = GRID_ROWS * GRID_COLS
    for marker_corners, marker_id in zip(corners, ids):
        if int(marker_id) >= card_id_limit:
            continue
        points = marker_corners.reshape(4, 2).astype(np.float32)
        center = tuple(np.mean(points, axis=0).astype(float))
        grid_info = aruco_grid_position(marker_id, center, frame.shape)
        x_min, y_min = np.min(points, axis=0)
        x_max, y_max = np.max(points, axis=0)
        records.append({
            'description': f'aruco_{int(marker_id)}', 'score': 1.0, 'class_index': int(marker_id),
            'aruco_id': int(marker_id), 'row': grid_info['row'], 'col': grid_info['col'],
            'cell': grid_info['cell'], 'center': grid_info['center'],
            'corners': [[float(x), float(y)] for x, y in points],
            'box': [float(y_min), float(x_min), float(y_max), float(x_max)],
            'crop_box': [float(y_min), float(x_min), float(y_max), float(x_max)],
            'detection_mode': detection_mode, 'detector': 'aruco',
        })
    return sorted(records, key=lambda record: (record['row'], record['col'], record['aruco_id']))


def aruco_records_for_grid_cell(frame, row, col):
    """Crop one grid cell, detect ArUco markers in that stretched crop, and map them back to the frame."""
    crop, src_quad, dst_quad = crop_grid_cell(frame, row, col)
    corners, ids = detect_aruco_corners_and_ids(crop)
    records = []
    for marker_corners, marker_id in zip(corners, ids):
        crop_points = marker_corners.reshape(4, 2).astype(np.float32)
        frame_points = crop_points_to_frame(crop_points, src_quad, dst_quad)
        center = tuple(np.mean(frame_points, axis=0).astype(float))
        grid_info = aruco_grid_position(marker_id, center, frame.shape)
        crop_x_min, crop_y_min = np.min(crop_points, axis=0)
        crop_x_max, crop_y_max = np.max(crop_points, axis=0)
        frame_x_min, frame_y_min = np.min(frame_points, axis=0)
        frame_x_max, frame_y_max = np.max(frame_points, axis=0)
        records.append({
            'description': f'aruco_{int(marker_id)}', 'score': 1.0, 'class_index': int(marker_id),
            'aruco_id': int(marker_id), 'row': grid_info['row'], 'col': grid_info['col'],
            'cell': grid_info['cell'], 'center': grid_info['center'],
            'corners': [[float(x), float(y)] for x, y in frame_points],
            'box': [float(frame_y_min), float(frame_x_min), float(frame_y_max), float(frame_x_max)],
            'crop_box': [float(crop_y_min), float(crop_x_min), float(crop_y_max), float(crop_x_max)],
            'detection_mode': 'grid', 'detector': 'aruco',
        })
    return records


def yolo_record_from_box(frame_shape, box, score, class_index, detection_mode, crop_box=None):
    y_min, x_min, y_max, x_max = map(float, box)
    center = ((x_min + x_max) / 2.0, (y_min + y_max) / 2.0)
    grid_info = grid_position_for_point(center, frame_shape)
    return {
        'description': yolo.class_names[int(class_index)], 'score': float(score), 'class_index': int(class_index),
        'row': grid_info['row'], 'col': grid_info['col'], 'cell': grid_info['cell'], 'center': grid_info['center'],
        'box': [y_min, x_min, y_max, x_max],
        'crop_box': [float(value) for value in (crop_box if crop_box is not None else box)],
        'detection_mode': detection_mode, 'detector': 'yolo',
    }


def best_detection_for_grid_cell(frame, row, col, score_thresh=None):
    """Run YOLO on one perspective-corrected grid cell and map the best box back to the frame."""
    crop, src_quad, dst_quad = crop_grid_cell(frame, row, col)
    boxes, scores, classes = yolo.run(crop, score_thresh=score_thresh)
    show_processed_grid_crop(crop, boxes, scores, classes, pos_name(row, col))
    if not scores.any():
        return None
    best_idx = int(np.argmax(scores))
    crop_box = boxes[best_idx]
    frame_box = crop_box_to_frame_box(crop_box, src_quad, dst_quad, frame.shape)
    record = yolo_record_from_box(frame.shape, frame_box, scores[best_idx], classes[best_idx], detection_mode='grid', crop_box=crop_box)
    record['row'] = int(row)
    record['col'] = int(col)
    record['cell'] = pos_name(row, col)
    record['center'] = crop_box_center_to_frame(crop_box, src_quad, dst_quad)
    return record


def yolo_records_for_grid(frame, score_thresh=None):
    records = []
    for row in range(GRID_ROWS):
        for col in range(GRID_COLS):
            record = best_detection_for_grid_cell(frame, row, col, score_thresh=score_thresh)
            if record is not None:
                records.append(record)
    return sorted(records, key=lambda record: record['score'], reverse=True)


def aruco_records_for_grid(frame):
    records = []
    for row in range(GRID_ROWS):
        for col in range(GRID_COLS):
            records.extend(aruco_records_for_grid_cell(frame, row, col))
    return sorted(records, key=lambda record: (record['row'], record['col'], record['aruco_id']))


def yolo_records_for_full_frame(frame, score_thresh=None):
    boxes, scores, classes = yolo.run(frame, score_thresh=score_thresh)
    if not scores.any():
        return []
    records = [
        yolo_record_from_box(frame.shape, box, score, class_index, detection_mode='full_frame')
        for box, score, class_index in zip(boxes, scores, classes)
    ]
    return sorted(records, key=lambda record: record['score'], reverse=True)


def record_selection_key(record):
    return (record.get('detector') == 'yolo', 'aruco_id' in record, record.get('score', 1.0))


def merge_aruco_with_yolo_records(yolo_records, aruco_records):
    """Attach nearest ArUco IDs to YOLO cards, optionally using marker IDs for stored positions."""
    merged_records = [dict(record) for record in yolo_records]
    used_aruco_indices = set()
    for yolo_record in merged_records:
        if not aruco_records:
            break
        yolo_center = np.array(yolo_record['center'], dtype=np.float32)
        candidate_indices = [i for i in range(len(aruco_records)) if i not in used_aruco_indices]
        if not candidate_indices:
            break
        best_index = min(
            candidate_indices,
            key=lambda index: np.linalg.norm(np.array(aruco_records[index]['center'], dtype=np.float32) - yolo_center),
        )
        aruco_record = aruco_records[best_index]
        aruco_center = tuple(map(float, aruco_record['center']))
        yolo_record['yolo_cell'] = yolo_record['cell']
        yolo_record['yolo_center'] = yolo_record['center']
        yolo_record['aruco_id'] = int(aruco_record['aruco_id'])
        yolo_record['aruco_score'] = float(aruco_record.get('score', 1.0))
        yolo_record['aruco_center'] = aruco_center
        yolo_record['corners'] = aruco_record['corners']
        if CARD_POSITION_MODE == 'aruco_id':
            grid_info = grid_position_for_aruco_id(aruco_record['aruco_id'], aruco_center)
            yolo_record['row'] = int(grid_info['row'])
            yolo_record['col'] = int(grid_info['col'])
            yolo_record['cell'] = grid_info['cell']
        used_aruco_indices.add(best_index)
    unmatched_aruco_records = [dict(r) for i, r in enumerate(aruco_records) if i not in used_aruco_indices]
    merged_records.extend(unmatched_aruco_records)
    return sorted(merged_records, key=record_selection_key, reverse=True)


def scan_grid_cells_for_objects(frame, score_thresh=None):
    """Crop/stretch each grid cell, then run YOLO and ArUco marker detection on each crop."""
    yolo_records = yolo_records_for_grid(frame, score_thresh=score_thresh)
    aruco_records = aruco_records_for_grid(frame)
    return merge_aruco_with_yolo_records(yolo_records, aruco_records)


def full_frame_records(frame, score_thresh=None):
    """Run YOLO and ArUco marker detection on the full camera frame."""
    yolo_records = yolo_records_for_full_frame(frame, score_thresh=score_thresh)
    aruco_records = aruco_records_for_frame(frame, detection_mode='full_frame')
    return merge_aruco_with_yolo_records(yolo_records, aruco_records)


ARUCO_CARD_CROP_SIZE = (416, 416)
ARUCO_CARD_CROP_MARGIN = 1.6


def crop_around_marker(frame, marker_corners):
    """Returns (crop, src_quad, dst_quad) for a fixed-size square region
    centered on one ArUco marker -- the adaptive-crop analog of
    crop_grid_cell() above, which crops a fixed geometric cell instead."""
    height, width = frame.shape[:2]
    points = np.asarray(marker_corners, dtype=np.float32).reshape(4, 2)
    center = np.mean(points, axis=0)
    marker_span = float(np.max(np.max(points, axis=0) - np.min(points, axis=0)))
    half = max(marker_span * ARUCO_CARD_CROP_MARGIN, 1.0)
    src = np.float32([
        [center[0] - half, center[1] - half], [center[0] + half, center[1] - half],
        [center[0] + half, center[1] + half], [center[0] - half, center[1] + half],
    ])
    src[:, 0] = np.clip(src[:, 0], 0, width - 1)
    src[:, 1] = np.clip(src[:, 1], 0, height - 1)
    crop_width, crop_height = ARUCO_CARD_CROP_SIZE
    dst = np.float32([[0, 0], [crop_width - 1, 0], [crop_width - 1, crop_height - 1], [0, crop_height - 1]])
    transform = cv2.getPerspectiveTransform(src, dst)
    crop = cv2.warpPerspective(frame, transform, (crop_width, crop_height))
    return crop, src, dst


def best_detection_for_aruco_marker(frame, aruco_record, score_thresh=None):
    """Run YOLO on one marker-centered crop and map the best box back to
    the frame -- the adaptive-crop analog of best_detection_for_grid_cell()
    above."""
    crop, src_quad, dst_quad = crop_around_marker(frame, aruco_record['corners'])
    boxes, scores, classes = yolo.run(crop, score_thresh=score_thresh)
    if not scores.any():
        return None
    best_idx = int(np.argmax(scores))
    crop_box = boxes[best_idx]
    frame_box = crop_box_to_frame_box(crop_box, src_quad, dst_quad, frame.shape)
    record = yolo_record_from_box(
        frame.shape, frame_box, scores[best_idx], classes[best_idx],
        detection_mode='aruco_card', crop_box=crop_box,
    )
    record['center'] = crop_box_center_to_frame(crop_box, src_quad, dst_quad)
    return record


def scan_aruco_markers_for_objects(frame, score_thresh=None):
    """One YOLO call per visible per-card marker, cropped tightly around it."""
    aruco_records = aruco_records_for_frame(frame, detection_mode='full_frame')
    yolo_records = []
    for aruco_record in aruco_records:
        record = best_detection_for_aruco_marker(frame, aruco_record, score_thresh=score_thresh)
        if record is not None:
            yolo_records.append(record)
    yolo_records.sort(key=lambda record: record['score'], reverse=True)
    return merge_aruco_with_yolo_records(yolo_records, aruco_records)


def records_for_current_mode(frame, score_thresh=None):
    """Detect face-up cards using the configured DETECTION_APPROACH -- the
    single entry point detect_position() calls below, regardless of which
    approach is active."""
    approach = DETECTION_APPROACH
    if approach == 'yolo_full_frame':
        return yolo_records_for_full_frame(frame, score_thresh=score_thresh)
    if approach == 'yolo_grid_crops':
        return scan_grid_cells_for_objects(frame, score_thresh=score_thresh)
    if approach == 'aruco_border':
        n = calibrate_from_border_markers(frame)
        if n < 4:
            print(f'[aruco_border] {n}/4 border markers visible -- using the last valid board calibration')
        return scan_grid_cells_for_objects(frame, score_thresh=score_thresh)
    if approach == 'aruco_per_card':
        yolo_records = yolo_records_for_full_frame(frame, score_thresh=score_thresh)
        aruco_records = aruco_records_for_frame(frame, detection_mode='full_frame')
        return merge_aruco_with_yolo_records(yolo_records, aruco_records)
    if approach == 'aruco_per_card_grid_crops':
        return scan_grid_cells_for_objects(frame, score_thresh=score_thresh)
    if approach == 'aruco_per_card_crop':
        return scan_aruco_markers_for_objects(frame, score_thresh=score_thresh)
    if approach == 'aruco_per_card_crop_verified':
        n = calibrate_from_border_markers(frame)
        if n < 4:
            print(f'[aruco_per_card_crop_verified] {n}/4 border markers visible -- using the last valid board calibration')
        return scan_aruco_markers_for_objects(frame, score_thresh=score_thresh)
    raise ValueError(f"Unknown DETECTION_APPROACH: {approach!r}. Choose from: {list(_APPROACH_SETTINGS)}")


def turn_score_thresholds(base_threshold=None, fallback_thresholds=None):
    """Base threshold followed by lower fallback thresholds, descending, deduplicated."""
    base_threshold = yolo.YOLO_SCORE_THRESHOLD if base_threshold is None else base_threshold
    if fallback_thresholds is None:
        fallback_thresholds = YOLO_FALLBACK_SCORE_THRESHOLDS
    thresholds = [float(base_threshold)]
    for threshold in sorted({float(v) for v in fallback_thresholds}, reverse=True):
        if threshold < base_threshold and not any(np.isclose(threshold, existing) for existing in thresholds):
            thresholds.append(threshold)
    return thresholds


def draw_detection_label(frame, label, x, y, color=(255, 255, 255)):
    text_size = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.6, 2)[0]
    y = max(y, text_size[1] + 8)
    cv2.rectangle(frame, (x, y - text_size[1] - 8), (x + text_size[0] + 8, y + 4), (0, 0, 0), -1)
    return cv2.putText(frame, label, (x + 4, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, color, 2, cv2.LINE_AA)


def record_color(record):
    if not len(yolo.colors):
        return (255, 255, 255)
    return yolo.colors[int(record.get('class_index', 0)) % len(yolo.colors)]


def draw_aruco_marker(frame, record, color):
    points = np.array(record['corners'], dtype=np.int32)
    cv2.polylines(frame, [points], True, color, 2)
    marker_center = record.get('aruco_center', record['center'])
    center_x, center_y = map(int, marker_center)
    cv2.circle(frame, (center_x, center_y), 4, color, -1)
    return frame


def marker_label(record):
    if 'aruco_id' in record:
        if record.get('detector') == 'yolo':
            return f"{record['description']} {record['score']:.2f} ID {record['aruco_id']} {record['cell']}"
        return f"ID {record['aruco_id']} {record['cell']}"
    return f"{record['description']} {record['score']:.2f} {record['cell']}"


def annotate_records_frame(frame, records, turn_label='Current detection'):
    annotated = draw_grid(frame)
    sorted_records = sorted(records, key=lambda record: record.get('score', 1.0), reverse=True)
    debug_records = sorted_records if TURN_DEBUG_SHOW_ALL_DETECTIONS else sorted_records[:MAX_OBJECTS]
    for object_number, record in enumerate(debug_records, start=1):
        y_min, x_min, y_max, x_max = map(int, record['box'])
        color = record_color(record)
        if record.get('detector') == 'yolo':
            annotated = cv2.rectangle(annotated, (x_min, y_min), (x_max, y_max), color, 2)
        if 'corners' in record:
            annotated = draw_aruco_marker(annotated, record, color)
        elif record.get('detector') != 'yolo':
            annotated = cv2.rectangle(annotated, (x_min, y_min), (x_max, y_max), color, 2)
        if TURN_DEBUG_SHOW_LABELS:
            label = f"{object_number}: {marker_label(record)}"
            annotated = draw_detection_label(annotated, label, x_min, y_min, color)
    marker_count = len([r for r in records if 'aruco_id' in r])
    yolo_count = len([r for r in records if r.get('detector') == 'yolo'])
    mode_word = 'grid mode' if DETECTION_MODE == 'grid' else 'full-frame mode'
    status = f"{turn_label}: {mode_word}, found {yolo_count} YOLO object(s), {marker_count} ArUco marker(s)"
    annotated = draw_detection_label(annotated, status, 20, 30, (255, 255, 255))
    if len(records) < MAX_OBJECTS:
        warning = 'Expected two revealed cards. Check YOLO threshold, ArUco dictionary, grid alignment, lighting, or visibility.'
        annotated = draw_detection_label(annotated, warning, 20, 65, (0, 255, 255))
    return annotated


def _set_image_widget(widget, frame):
    ok, encoded = cv2.imencode('.jpeg', frame)
    if not ok:
        raise RuntimeError('Could not encode debug image.')
    widget.value = encoded.tobytes()


def show_detection_frame(frame, records, turn_label='Current detection'):
    """Update the debug/detection.jpg preview without creating a new output."""
    annotated = annotate_records_frame(frame, records, turn_label=turn_label)
    if detection_image_widget is not None:
        _set_image_widget(detection_image_widget, annotated)


def annotate_processed_grid_crop(crop, boxes, scores, classes, cell):
    """Draw crop-space YOLO results on the exact 416x416 input image."""
    annotated = crop.copy()
    for box, score, class_idx in zip(boxes, scores, classes):
        y_min, x_min, y_max, x_max = map(int, box)
        color = yolo.colors[int(class_idx) % len(yolo.colors)]
        cv2.rectangle(annotated, (x_min, y_min), (x_max, y_max), color, 2)
        label = f'{yolo.class_names[int(class_idx)]} {float(score):.2f}'
        annotated = draw_detection_label(annotated, label, x_min, max(18, y_min), color)
    summary = f'{cell} processed 416x416 crop: {len(boxes)} detection(s)'
    return draw_detection_label(annotated, summary, 8, 22, (255, 255, 255))


def show_processed_grid_crop(crop, boxes, scores, classes, cell):
    """Keep debug/processed_crop.jpg fixed while replacing only its content."""
    annotated = annotate_processed_grid_crop(crop, boxes, scores, classes, cell)
    if processed_crop_image_widget is not None:
        _set_image_widget(processed_crop_image_widget, annotated)


def print_aruco_marker_report(records):
    marker_records = [r for r in records if 'aruco_id' in r]
    if not marker_records:
        print('No ArUco markers detected')
        return
    print('ArUco markers detected:')
    for record in sorted(marker_records, key=lambda item: (item['row'], item['col'], item['aruco_id'])):
        center_x, center_y = record.get('aruco_center', record['center'])
        print(f"  id={record['aruco_id']} cell={record['cell']} row={record['row'] + 1} col={record['col'] + 1} center=({center_x:.1f}, {center_y:.1f})")


# -- Section 5: camera -------------------------------------------------------

def video_devices():
    devices = []
    for name in os.listdir('/dev'):
        if name.startswith('video') and name[5:].isdigit():
            devices.append((int(name[5:]), f'/dev/{name}'))
    return [device for _, device in sorted(devices)]


def open_camera(device):
    camera = cv2.VideoCapture(device, cv2.CAP_V4L2)
    camera.set(cv2.CAP_PROP_BUFFERSIZE, CAMERA_BUFFER_SIZE)
    camera.set(cv2.CAP_PROP_FOURCC, cv2.VideoWriter_fourcc(*'MJPG'))
    camera.set(cv2.CAP_PROP_FRAME_WIDTH, CAMERA_SIZE[0])
    camera.set(cv2.CAP_PROP_FRAME_HEIGHT, CAMERA_SIZE[1])
    camera.set(cv2.CAP_PROP_FPS, CAMERA_FPS)
    return camera


def scan_working_camera():
    for device in video_devices():
        camera = open_camera(device)
        ret, _ = camera.read()
        if camera.isOpened() and ret:
            print(f'Using camera from {device}')
            return camera, device
        camera.release()
    raise RuntimeError('Could not find a working /dev/video* camera.')


def camera_is_open():
    return cap is not None and cap.isOpened()


def reopen_camera():
    global cap, camera_device
    if camera_is_open():
        cap.release()
    cap, camera_device = scan_working_camera()
    return cap


def ensure_camera_open():
    if not camera_is_open():
        reopen_camera()
    return cap


def drain_camera_buffer(flush_frames=8):
    ensure_camera_open()
    for _ in range(max(0, flush_frames)):
        cap.grab()


def read_camera_frame(read_attempts=8):
    ensure_camera_open()
    for _ in range(max(1, read_attempts)):
        ret, frame = cap.read()
        if ret and frame is not None:
            return frame
        cap.grab()
        ret, frame = cap.retrieve()
        if ret and frame is not None:
            return frame
        time.sleep(0.05)
    return None


def capture_frame(flush_frames=8, settle_seconds=0.3, read_attempts=8):
    if settle_seconds > 0:
        time.sleep(settle_seconds)
    for attempt in range(2):
        drain_camera_buffer(flush_frames if attempt == 0 else 0)
        frame = read_camera_frame(read_attempts=read_attempts)
        if frame is not None:
            return frame
        reopen_camera()
    raise RuntimeError('Could not read a fresh frame from the camera.')


def camera_alignment_snapshot():
    """Show the full board in marker-defined canonical orientation."""
    frame = capture_frame(flush_frames=1, settle_seconds=0.0)
    marker_count = calibrate_from_border_markers(frame)
    if marker_count < 4:
        raise RuntimeError(f'Only {marker_count}/4 border markers visible; cannot orient the board.')
    oriented = draw_canonical_board(frame)
    _set_image_widget(alignment_image_widget, oriented)
    return oriented


# -- Section 6: per-position detection --------------------------------------

def detect_position(pos, show=True):
    """Auto-calibrate from border markers, crop only `pos`, and run YOLO on
    that perspective-corrected 416x416 cell. Retries fresh photos at lower
    confidence thresholds without ever falling back to full-frame inference."""
    row, col = parse_pos(pos)
    if not (0 <= row < GRID_ROWS and 0 <= col < GRID_COLS):
        raise ValueError(f'Position outside configured grid: {pos!r}')

    for threshold in turn_score_thresholds():
        for photo in range(DETECT_PHOTO_COUNT):
            frame = capture_frame(
                flush_frames=DETECT_FLUSH_FRAMES if photo == 0 else 0,
                settle_seconds=DETECT_SETTLE_SECONDS if photo == 0 else 0.15,
            )
            marker_count = calibrate_from_border_markers(frame)
            if marker_count < 4 and BOARD_CORNERS is None:
                print(f'[aruco_border] {marker_count}/4 border markers visible -- cannot crop {pos} yet')
                continue
            record = best_detection_for_grid_cell(frame, row, col, score_thresh=threshold)
            if record is not None:
                if show:
                    show_detection_frame(frame, [record], turn_label=f'{pos} grid-cell crop')
                return record['description'], record.get('score', 1.0)

    print(f'[detect] WARNING: no object detected in the {pos} grid-cell crop even at the lowest threshold.')
    if show:
        frame = capture_frame(flush_frames=0, settle_seconds=0.0)
        show_detection_frame(frame, [], turn_label=f'{pos} grid-cell crop')
    return 'unknown', 0.0


def detect_position_debug(pos):
    """Manual, one-off test of detection at a position. Does not touch
    match state or send anything to the referee -- setup/debug only."""
    cls, score = detect_position(pos, show=True)
    print(f'{pos}: {cls} (score={score:.2f})')
    return cls, score


def camera_debug_one_shot(pos='A1'):
    """Auto-orient the board from corner-marker roles and run YOLO on one cell."""
    pos = str(pos).strip().upper()
    row, col = parse_pos(pos)
    if not (0 <= row < GRID_ROWS and 0 <= col < GRID_COLS):
        raise ValueError(f'Position outside configured grid: {pos!r}')

    frame = capture_frame(flush_frames=4, settle_seconds=0.2)
    marker_count = calibrate_from_border_markers(frame)
    if marker_count < 4:
        raise RuntimeError(f'Only {marker_count}/4 border markers visible; cannot orient the board.')

    crop, _, _ = crop_grid_cell(frame, row, col)
    boxes, scores, classes = yolo.run(crop)

    board_view = draw_canonical_board(frame, selected_pos=pos)
    _set_image_widget(alignment_image_widget, board_view)
    show_processed_grid_crop(crop, boxes, scores, classes, pos)

    print(f'{pos}: {len(boxes)} object(s) detected in the processed grid-cell crop:')
    for score, class_idx in sorted(zip(scores, classes), reverse=True):
        print(f'  {yolo.class_names[int(class_idx)]}: {float(score):.2f}')
    return boxes, scores, classes


def camera_capture_test():
    """Grab one raw camera frame with no ArUco/YOLO involved -- confirms
    the camera itself is working before trusting the detection pipeline."""
    frame = capture_frame(flush_frames=1, settle_seconds=0.0)
    _set_image_widget(detection_image_widget, frame)
    return frame
