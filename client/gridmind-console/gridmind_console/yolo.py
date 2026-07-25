"""YOLO detection helpers -- console port of notebook Section 3, plus the
DPU-facing run() from the top of Section 4a's original cell.

Kept as module-level state (not a class) deliberately, mirroring the
notebook's own global-variable style, since match_client.py and mnist_hint.py
both need to call reload()/run() freely the same way the notebook's cells did.
"""
import colorsys
import random

import numpy as np
import cv2

# Lower this if detections are consistently missed; raise it if you get false positives.
YOLO_SCORE_THRESHOLD = 0.2

_ANCHOR_LIST = [10, 13, 16, 30, 33, 23, 30, 61, 62, 45, 59, 119, 116, 90, 156, 198, 373, 326]
anchors = np.array([float(x) for x in _ANCHOR_LIST]).reshape(-1, 2)

class_names = []
colors = []

_overlay = None
_model_path = None
dpu = None
inputTensors = None
outputTensors = None
shapeIn = None
shapeOut0 = shapeOut1 = shapeOut2 = None
input_data = None
output_data = None
image_buffer = None


def get_class(classes_path):
    with open(classes_path) as f:
        return [c.strip() for c in f.readlines()]


def _build_colors(names):
    num_classes = len(names)
    hsv_tuples = [(1.0 * x / num_classes, 1.0, 1.0) for x in range(num_classes)]
    palette = [tuple(int(c * 255) for c in colorsys.hsv_to_rgb(*hsv)) for hsv in hsv_tuples]
    random.seed(0)
    random.shuffle(palette)
    random.seed(None)
    return palette


def init(overlay, model_path, classes_path):
    """Loads the YOLO model onto the DPU and builds class_names/colors --
    console equivalent of running Section 3's cell + the top of Section 4a's
    cell in the notebook."""
    global _overlay, _model_path, class_names, colors
    global dpu, inputTensors, outputTensors, shapeIn, shapeOut0, shapeOut1, shapeOut2
    global input_data, output_data, image_buffer

    _overlay = overlay
    _model_path = str(model_path)
    class_names = get_class(classes_path)
    colors = _build_colors(class_names)

    overlay.load_model(_model_path)
    dpu = overlay.runner
    inputTensors = dpu.get_input_tensors()
    outputTensors = dpu.get_output_tensors()
    shapeIn = tuple(inputTensors[0].dims)
    shapeOut0, shapeOut1, shapeOut2 = (tuple(t.dims) for t in outputTensors)

    input_data = [np.empty(shapeIn, dtype=np.float32, order='C')]
    output_data = [
        np.empty(shapeOut0, dtype=np.float32, order='C'),
        np.empty(shapeOut1, dtype=np.float32, order='C'),
        np.empty(shapeOut2, dtype=np.float32, order='C'),
    ]
    image_buffer = input_data[0]


def reload():
    """Reloads the YOLO model after an MNIST capture swapped the DPU over --
    call this before any further board detection. Same overlay/model_path
    init() was originally called with."""
    global dpu, inputTensors, outputTensors, shapeIn, shapeOut0, shapeOut1, shapeOut2
    global input_data, output_data, image_buffer
    _overlay.load_model(_model_path)
    dpu = _overlay.runner
    inputTensors = dpu.get_input_tensors()
    outputTensors = dpu.get_output_tensors()
    shapeIn = tuple(inputTensors[0].dims)
    shapeOut0, shapeOut1, shapeOut2 = (tuple(t.dims) for t in outputTensors)
    input_data = [np.empty(shapeIn, dtype=np.float32, order='C')]
    output_data = [
        np.empty(shapeOut0, dtype=np.float32, order='C'),
        np.empty(shapeOut1, dtype=np.float32, order='C'),
        np.empty(shapeOut2, dtype=np.float32, order='C'),
    ]
    image_buffer = input_data[0]


def letterbox_image(image, size):
    ih, iw, _ = image.shape
    w, h = size
    scale = min(w / iw, h / ih)
    nw, nh = int(iw * scale), int(ih * scale)

    image = cv2.resize(image, (nw, nh), interpolation=cv2.INTER_LINEAR)
    new_image = np.ones((h, w, 3), np.uint8) * 128
    h_start, w_start = (h - nh) // 2, (w - nw) // 2
    new_image[h_start:h_start + nh, w_start:w_start + nw, :] = image
    return new_image


def pre_process(image, model_image_size):
    image = image[..., ::-1]
    boxed_image = letterbox_image(image, tuple(reversed(model_image_size)))
    image_data = np.array(boxed_image, dtype='float32') / 255.0
    return np.expand_dims(image_data, 0)


def _get_feats(feats, anchors_, num_classes, input_shape):
    num_anchors = len(anchors_)
    anchors_tensor = np.reshape(np.array(anchors_, dtype=np.float32), [1, 1, 1, num_anchors, 2])
    grid_size = np.shape(feats)[1:3]
    nu = num_classes + 5
    predictions = np.reshape(feats, [-1, grid_size[0], grid_size[1], num_anchors, nu])
    grid_y = np.tile(np.reshape(np.arange(grid_size[0]), [-1, 1, 1, 1]), [1, grid_size[1], 1, 1])
    grid_x = np.tile(np.reshape(np.arange(grid_size[1]), [1, -1, 1, 1]), [grid_size[0], 1, 1, 1])
    grid = np.array(np.concatenate([grid_x, grid_y], axis=-1), dtype=np.float32)

    box_xy = (1 / (1 + np.exp(-predictions[..., :2])) + grid) / np.array(grid_size[::-1], dtype=np.float32)
    box_wh = np.exp(predictions[..., 2:4]) * anchors_tensor / np.array(input_shape[::-1], dtype=np.float32)
    box_confidence = 1 / (1 + np.exp(-predictions[..., 4:5]))
    box_class_probs = 1 / (1 + np.exp(-predictions[..., 5:]))
    return box_xy, box_wh, box_confidence, box_class_probs


def correct_boxes(box_xy, box_wh, input_shape, image_shape):
    box_yx, box_hw = box_xy[..., ::-1], box_wh[..., ::-1]
    input_shape = np.array(input_shape, dtype=np.float32)
    image_shape = np.array(image_shape, dtype=np.float32)
    new_shape = np.around(image_shape * np.min(input_shape / image_shape))
    offset = (input_shape - new_shape) / 2.0 / input_shape
    scale = input_shape / new_shape
    box_yx = (box_yx - offset) * scale
    box_hw *= scale

    box_mins, box_maxes = box_yx - box_hw / 2.0, box_yx + box_hw / 2.0
    boxes = np.concatenate([box_mins[..., 0:1], box_mins[..., 1:2], box_maxes[..., 0:1], box_maxes[..., 1:2]], axis=-1)
    boxes *= np.concatenate([image_shape, image_shape], axis=-1)
    return boxes


def boxes_and_scores(feats, anchors_, classes_num, input_shape, image_shape):
    box_xy, box_wh, box_confidence, box_class_probs = _get_feats(feats, anchors_, classes_num, input_shape)
    boxes = np.reshape(correct_boxes(box_xy, box_wh, input_shape, image_shape), [-1, 4])
    box_scores = np.reshape(box_confidence * box_class_probs, [-1, classes_num])
    return boxes, box_scores


def nms_boxes(boxes, scores):
    x1, y1, x2, y2 = boxes[:, 0], boxes[:, 1], boxes[:, 2], boxes[:, 3]
    areas = (x2 - x1 + 1) * (y2 - y1 + 1)
    order = scores.argsort()[::-1]
    keep = []
    while order.size > 0:
        i = order[0]
        keep.append(i)
        xx1, yy1 = np.maximum(x1[i], x1[order[1:]]), np.maximum(y1[i], y1[order[1:]])
        xx2, yy2 = np.minimum(x2[i], x2[order[1:]]), np.minimum(y2[i], y2[order[1:]])
        inter = np.maximum(0.0, xx2 - xx1 + 1) * np.maximum(0.0, yy2 - yy1 + 1)
        ovr = inter / (areas[i] + areas[order[1:]] - inter)
        order = order[np.where(ovr <= 0.55)[0] + 1]
    return keep


def evaluate(yolo_outputs, image_shape, names, anchors_, score_thresh=None):
    score_thresh = YOLO_SCORE_THRESHOLD if score_thresh is None else score_thresh
    anchor_mask = [[6, 7, 8], [3, 4, 5], [0, 1, 2]]
    boxes, box_scores = [], []
    input_shape = np.array(np.shape(yolo_outputs[0])[1:3]) * 32

    for i in range(len(yolo_outputs)):
        b, s = boxes_and_scores(yolo_outputs[i], anchors_[anchor_mask[i]], len(names), input_shape, image_shape)
        boxes.append(b)
        box_scores.append(s)

    boxes = np.concatenate(boxes, axis=0)
    box_scores = np.concatenate(box_scores, axis=0)
    mask = box_scores >= score_thresh
    boxes_, scores_, classes_ = [], [], []

    for c in range(len(names)):
        class_boxes = boxes[mask[:, c]]
        class_scores = box_scores[:, c][mask[:, c]]
        keep = nms_boxes(class_boxes, class_scores)
        boxes_.append(class_boxes[keep])
        scores_.append(class_scores[keep])
        classes_.append(np.ones(len(keep), dtype=np.int32) * c)

    if not boxes_:
        return np.array([]), np.array([]), np.array([])
    return np.concatenate(boxes_, axis=0), np.concatenate(scores_, axis=0), np.concatenate(classes_, axis=0)


def run(frame, score_thresh=None):
    image_size = frame.shape[:2]
    image_data = np.array(pre_process(frame, (416, 416)), dtype=np.float32)
    image_buffer[0, ...] = image_data.reshape(shapeIn[1:])

    job_id = dpu.execute_async(input_data, output_data)
    dpu.wait(job_id)

    yolo_outputs = [
        np.reshape(output_data[0], shapeOut0),
        np.reshape(output_data[1], shapeOut1),
        np.reshape(output_data[2], shapeOut2),
    ]
    return evaluate(yolo_outputs, image_size, class_names, anchors, score_thresh=score_thresh)
