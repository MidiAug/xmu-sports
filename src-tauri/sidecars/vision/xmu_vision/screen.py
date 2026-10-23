"""Screenshot helpers (window-relative)."""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

import numpy as np
from PIL import Image

if TYPE_CHECKING:
    from xmu_vision.window import WindowInfo


def grab_bbox_bgr(left: int, top: int, width: int, height: int) -> np.ndarray:
    """Return BGR uint8 array for OpenCV."""
    import mss

    with mss.mss() as sct:
        shot = sct.grab({"left": left, "top": top, "width": width, "height": height})
        bgra = np.array(shot)
        return np.ascontiguousarray(bgra[:, :, :3])


def grab_window(win: "WindowInfo") -> np.ndarray:
    left, top, width, height = win.bbox
    if width <= 0 or height <= 0:
        raise RuntimeError(f"invalid window size: {win}")
    return grab_bbox_bgr(left, top, width, height)


def save_bgr(path: str | Path, image_bgr: np.ndarray) -> Path:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    rgb = image_bgr[:, :, ::-1]
    Image.fromarray(rgb).save(path)
    return path
