"""Template match + optional OCR text locate."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np


@dataclass(frozen=True)
class Match:
    x: int  # center x in image coords
    y: int  # center y
    score: float
    label: str = ""


def match_template(
    haystack_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    *,
    threshold: float = 0.82,
) -> Match | None:
    if needle_bgr.shape[0] > haystack_bgr.shape[0] or needle_bgr.shape[1] > haystack_bgr.shape[1]:
        return None
    res = cv2.matchTemplate(haystack_bgr, needle_bgr, cv2.TM_CCOEFF_NORMED)
    _min_val, max_val, _min_loc, max_loc = cv2.minMaxLoc(res)
    if max_val < threshold:
        return None
    h, w = needle_bgr.shape[:2]
    cx = int(max_loc[0] + w / 2)
    cy = int(max_loc[1] + h / 2)
    return Match(cx, cy, float(max_val))


def load_template(path: str | Path) -> np.ndarray:
    data = np.fromfile(str(path), dtype=np.uint8)
    img = cv2.imdecode(data, cv2.IMREAD_COLOR)
    if img is None:
        raise FileNotFoundError(f"cannot decode template: {path}")
    return img


def match_template_file(
    haystack_bgr: np.ndarray,
    template_path: str | Path,
    *,
    threshold: float = 0.82,
) -> Match | None:
    needle = load_template(template_path)
    m = match_template(haystack_bgr, needle, threshold=threshold)
    if m:
        return Match(m.x, m.y, m.score, label=str(template_path))
    return None


_ocr_engine = None


def _get_ocr():
    global _ocr_engine
    if _ocr_engine is not None:
        return _ocr_engine
    try:
        from rapidocr_onnxruntime import RapidOCR  # type: ignore

        _ocr_engine = RapidOCR()
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError(
            "OCR 未安装。请: pip install rapidocr-onnxruntime\n"
            f"原始错误: {exc}"
        ) from exc
    return _ocr_engine


@dataclass(frozen=True)
class OcrHit:
    text: str
    x: int
    y: int
    score: float
    box: tuple[tuple[float, float], ...]


def ocr_image(image_bgr: np.ndarray) -> list[OcrHit]:
    engine = _get_ocr()
    rgb = cv2.cvtColor(image_bgr, cv2.COLOR_BGR2RGB)
    result, _ = engine(rgb)
    hits: list[OcrHit] = []
    if not result:
        return hits
    for item in result:
        # item: [box, text, score]
        box, text, score = item[0], item[1], float(item[2])
        xs = [p[0] for p in box]
        ys = [p[1] for p in box]
        cx = int(sum(xs) / len(xs))
        cy = int(sum(ys) / len(ys))
        hits.append(OcrHit(str(text), cx, cy, score, tuple((float(p[0]), float(p[1])) for p in box)))
    return hits


def find_text(
    image_bgr: np.ndarray,
    needle: str,
    *,
    min_score: float = 0.5,
    substring: bool = True,
) -> Match | None:
    needle = needle.strip()
    if not needle:
        return None
    best: OcrHit | None = None
    for hit in ocr_image(image_bgr):
        if hit.score < min_score:
            continue
        ok = (needle in hit.text) if substring else (hit.text.strip() == needle)
        if not ok:
            continue
        if best is None or hit.score > best.score:
            best = hit
    if best is None:
        return None
    return Match(best.x, best.y, best.score, label=best.text)
