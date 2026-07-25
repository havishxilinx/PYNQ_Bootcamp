"""Match configuration -- console equivalent of the notebook's Section 2 cell.

Operators edit config.json instead of a Python cell. Every field here has
the exact same meaning and default as the corresponding notebook constant.
"""
import json
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Optional, List


@dataclass
class Config:
    server: str = "192.168.1.100:5000"
    broker_key: str = "bootcamp2024"
    referee_id: str = "arena-1-referee"
    master_id: str = "master-referee"
    team_name: str = "red"
    team_secret: str = ""
    board_id_override: str = ""

    grid_rows: int = 5
    grid_cols: int = 6

    # Perspective-correction corners for a fixed/angled camera, used only
    # when no ArUco border markers are configured/visible. Four [x, y]
    # points: top-left, top-right, bottom-right, bottom-left. None means
    # "use the raw frame corners" (a head-on camera).
    board_corners: Optional[List[List[float]]] = None

    # One of: yolo_full_frame, yolo_grid_crops, aruco_border, aruco_per_card,
    # aruco_per_card_grid_crops, aruco_per_card_crop, aruco_per_card_crop_verified
    detection_approach: str = "aruco_border"

    border_marker_tl: int = 30
    border_marker_tr: int = 31
    border_marker_br: int = 33
    border_marker_bl: int = 32

    notebook_dir: str = "."
    model_path: str = "tf_yolov3_voc.xmodel"
    classes_path: str = "img/voc_classes.txt"

    genesis_admin_password: str = ""

    def __post_init__(self):
        if self.board_corners is not None:
            assert len(self.board_corners) == 4, "board_corners must have exactly 4 [x, y] points"

    @classmethod
    def load(cls, path):
        path = Path(path)
        if not path.exists():
            raise FileNotFoundError(
                f"Config file not found: {path}. Copy config.example.json to "
                f"config.json and fill in your team's values first."
            )
        data = json.loads(path.read_text())
        known = {f.name for f in cls.__dataclass_fields__.values()}
        unknown = set(data) - known
        if unknown:
            raise ValueError(f"Unknown config key(s) in {path}: {sorted(unknown)}")
        return cls(**data)

    def save(self, path):
        Path(path).write_text(json.dumps(asdict(self), indent=2) + "\n")
