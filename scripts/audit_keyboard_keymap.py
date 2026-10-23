"""Audit soft-keyboard key boxes on images/14.png (mirrors vision-core keyboard.rs).

Draws: 10 digits + 26 letters (QWERTY snap from white caps) + 取消/完成/退格.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SRC = ROOT / "images" / "keymap-audit" / "ref-clean.png"
FALLBACK_SRC = ROOT / "images" / "14.png"
OUT_DIR = ROOT / "images" / "keymap-audit"
KEY_CAP_V_MIN = 248
KEY_CAP_S_MAX = 45
CAP_NMS_IOU = 0.30
QWERTY_ROWS = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"]


@dataclass
class BBox:
    x0: int
    y0: int
    x1: int
    y1: int

    def w(self) -> int:
        return max(0, self.x1 - self.x0)

    def h(self) -> int:
        return max(0, self.y1 - self.y0)

    def cx(self) -> int:
        return (self.x0 + self.x1) // 2

    def cy(self) -> int:
        return (self.y0 + self.y1) // 2

    def area(self) -> int:
        return self.w() * self.h()


def opencv_hsv(b: int, g: int, r: int) -> tuple[float, float, float]:
    # Match vision-core opencv_hsv (OpenCV 0-180 H, 0-255 S/V).
    mx = max(r, g, b)
    mn = min(r, g, b)
    v = float(mx)
    delta = float(mx - mn)
    s = 0.0 if mx == 0 else delta * 255.0 / mx
    if delta == 0:
        h = 0.0
    elif mx == r:
        h = 60.0 * (((g - b) / delta) % 6.0)
    elif mx == g:
        h = 60.0 * (((b - r) / delta) + 2.0)
    else:
        h = 60.0 * (((r - g) / delta) + 4.0)
    return h / 2.0, s, v


def pixel_is_key_cap(b: int, g: int, r: int) -> bool:
    _h, s, v = opencv_hsv(b, g, r)
    return s <= KEY_CAP_S_MAX and v >= KEY_CAP_V_MIN


def box_iou(a: BBox, b: BBox) -> float:
    ix0, iy0 = max(a.x0, b.x0), max(a.y0, b.y0)
    ix1, iy1 = min(a.x1, b.x1), min(a.y1, b.y1)
    iw, ih = max(0, ix1 - ix0), max(0, iy1 - iy0)
    inter = iw * ih
    if inter <= 0:
        return 0.0
    union = a.area() + b.area() - inter
    return inter / union if union > 0 else 0.0


def nms_caps(comps: list[tuple[BBox, int]], thr: float) -> list[BBox]:
    comps = sorted(comps, key=lambda t: t[1], reverse=True)
    kept: list[BBox] = []
    for b, _ in comps:
        if any(box_iou(b, k) >= thr for k in kept):
            continue
        kept.append(b)
    return kept


def white_key_caps(bgr: np.ndarray, body_top: int, body_bottom: int) -> list[BBox]:
    h, w = bgr.shape[:2]
    x0 = int(round(w * 0.02))
    x1 = int(round(w * 0.98))
    y0 = max(0, body_top)
    y1 = min(h, body_bottom)
    if x1 <= x0 + 8 or y1 <= y0 + 8:
        return []

    mask = np.zeros((h, w), np.uint8)
    roi = bgr[y0:y1, x0:x1]
    # Vectorized-ish loop on ROI
    for yy in range(roi.shape[0]):
        row = roi[yy]
        for xx in range(roi.shape[1]):
            b, g, r = int(row[xx, 0]), int(row[xx, 1]), int(row[xx, 2])
            if pixel_is_key_cap(b, g, r):
                mask[y0 + yy, x0 + xx] = 1

    min_area = int(np.clip(w * h * 0.00035, 80, 400))
    max_area = int(np.clip(w * h * 0.012, 800, 6000))
    num, labels, stats, _ = cv2.connectedComponentsWithStats(mask, connectivity=4)
    comps: list[tuple[BBox, int]] = []
    for i in range(1, num):
        area = int(stats[i, cv2.CC_STAT_AREA])
        if area < min_area or area > max_area:
            continue
        bx = int(stats[i, cv2.CC_STAT_LEFT])
        by = int(stats[i, cv2.CC_STAT_TOP])
        bw = int(stats[i, cv2.CC_STAT_WIDTH])
        bh = int(stats[i, cv2.CC_STAT_HEIGHT])
        if bw < 12 or bh < 12:
            continue
        aspect = bw / max(1, bh)
        if not (0.55 <= aspect <= 2.2):
            continue
        comps.append((BBox(bx, by, bx + bw, by + bh), area))
    return nms_caps(comps, CAP_NMS_IOU)


def cluster_key_rows(caps: list[BBox]) -> list[list[BBox]]:
    if not caps:
        return []
    sorted_caps = sorted(caps, key=lambda b: (b.cy(), b.cx()))
    hs = sorted(b.h() for b in sorted_caps)
    med_h = max(16, hs[len(hs) // 2])
    row_tol = int(round(med_h * 0.55))
    rows: list[list[BBox]] = []
    for b in sorted_caps:
        if rows:
            row_cy = sum(x.cy() for x in rows[-1]) // len(rows[-1])
            if abs(b.cy() - row_cy) <= row_tol:
                rows[-1].append(b)
                continue
        rows.append([b])
    for row in rows:
        row.sort(key=lambda b: b.cx())
    if len(rows) > 4:
        rows.sort(key=lambda r: len(r), reverse=True)
        rows = rows[:4]
        rows.sort(key=lambda r: sum(b.cy() for b in r) // max(1, len(r)))
    return rows


def snap_row_to_grid(row_boxes: list[BBox], n: int, frame_w: int) -> list[BBox]:
    if n == 0 or not row_boxes:
        return []
    areas = sorted(b.area() for b in row_boxes)
    med_a = max(1, areas[len(areas) // 2])
    solid = [b for b in row_boxes if b.area() >= int(med_a * 0.55)]
    if len(solid) < 3:
        solid = list(row_boxes)
    # Z-row often picks up the white glyph inside the pink delete key → drop extras on the right.
    if len(solid) > n:
        solid = sorted(solid, key=lambda b: b.cx())[:n]
    cys = sorted(b.cy() for b in solid)
    hs = sorted(b.h() for b in solid)
    ws = sorted(b.w() for b in solid)
    cy = float(cys[len(cys) // 2])
    kh = float(max(12, hs[len(hs) // 2]))
    kw = float(max(12, ws[len(ws) // 2]))
    cxs = sorted(float(b.cx()) for b in solid)

    if len(cxs) >= max(2, n - 1):
        pitch = (cxs[-1] - cxs[0]) / max(1, n - 1)
        left_cx = cxs[0]
        pitch = max(pitch, kw * 0.8)
    else:
        pitches = [cxs[i + 1] - cxs[i] for i in range(len(cxs) - 1)]
        pitches = [p for p in pitches if p > kw * 0.6]
        pitches.sort()
        pitch = (frame_w / 10.0) if not pitches else pitches[len(pitches) // 2]
        if cxs[0] < frame_w * 0.22:
            left_cx = cxs[0]
        else:
            row_w = pitch * (n - 1)
            left_cx = (frame_w - row_w) * 0.5
        pitch = max(pitch, kw * 0.8)

    half_w = kw * 0.48
    half_h = kh * 0.48
    out: list[BBox] = []
    for i in range(n):
        cx = left_cx + i * pitch
        out.append(
            BBox(
                int(round(cx - half_w)),
                int(round(cy - half_h)),
                int(round(cx + half_w)),
                int(round(cy + half_h)),
            )
        )
    return out


def estimate_keyboard_band(bgr: np.ndarray) -> tuple[int, int, int, list[BBox]]:
    """Without OCR: densest white-cap band → body; chrome sits just above digit row."""
    h, w = bgr.shape[:2]
    body_bottom = h - max(4, int(h * 0.015))
    best_top = int(h * 0.55)
    best_score = -1
    best_caps: list[BBox] = []
    # Prefer a top that still yields ≥20 caps (letter+digit), not an empty form band.
    for trial_top in range(int(h * 0.48), int(h * 0.78), 3):
        caps = white_key_caps(bgr, trial_top, body_bottom)
        score = len(caps)
        if score > best_score:
            best_score = score
            best_top = trial_top
            best_caps = caps
    rows = cluster_key_rows(best_caps)
    if rows:
        digit_top = min(b.y0 for b in rows[0])
        body_top = max(int(h * 0.45), digit_top - 6)
        chrome_cy = max(int(h * 0.42), body_top - 28)
    elif best_caps:
        min_cap_y = min(b.y0 for b in best_caps)
        body_top = max(int(h * 0.45), min_cap_y - 8)
        chrome_cy = max(int(h * 0.42), body_top - 28)
    else:
        body_top = best_top
        chrome_cy = max(int(h * 0.42), body_top - 28)
    caps = white_key_caps(bgr, body_top, body_bottom)
    return body_top, body_bottom, chrome_cy, caps


def find_backspace_box(bgr: np.ndarray, body_top: int) -> BBox | None:
    h, w = bgr.shape[:2]
    y0 = max(body_top, int(h * 0.55))
    roi = bgr[y0:h, int(w * 0.55) : w]
    if roi.size == 0:
        return None
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    mask = cv2.bitwise_or(
        cv2.inRange(hsv, (0, 80, 80), (12, 255, 255)),
        cv2.inRange(hsv, (165, 80, 80), (180, 255, 255)),
    )
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, np.ones((3, 3), np.uint8), iterations=2)
    cnts, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    best = None
    best_score = -1.0
    for c in cnts:
        a = float(cv2.contourArea(c))
        if a < 80:
            continue
        x, y, bw, bh = cv2.boundingRect(c)
        cx = int(w * 0.55) + x + bw // 2
        cy = y0 + y + bh // 2
        score = a + cx * 0.5 + cy * 0.3
        if score > best_score:
            best_score = score
            best = BBox(int(w * 0.55) + x, y0 + y, int(w * 0.55) + x + bw, y0 + y + bh)
    return best


def find_done_box(bgr: np.ndarray, chrome_cy: int) -> BBox | None:
    """Green「完成」— union ink, then expand left to full two-glyph hit box."""
    h, w = bgr.shape[:2]
    y0 = max(0, chrome_cy - 36)
    y1 = min(h, chrome_cy + 36)
    x_left = int(w * 0.55)
    strip = bgr[y0:y1, x_left:w]
    if strip.size == 0:
        return None
    hsv = cv2.cvtColor(strip, cv2.COLOR_BGR2HSV)
    mask = cv2.inRange(hsv, (35, 30, 60), (100, 255, 255))
    k = cv2.getStructuringElement(cv2.MORPH_RECT, (28, 8))
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, k, iterations=2)
    ys, xs = np.where(mask > 0)
    if len(xs) == 0:
        ww = max(56, int(w * 0.12))
        hh = 26
        x1 = w - int(w * 0.03)
        return BBox(x1 - ww, max(0, chrome_cy - hh // 2), x1, chrome_cy + hh // 2)
    ink = BBox(x_left + int(xs.min()), y0 + int(ys.min()), x_left + int(xs.max()) + 1, y0 + int(ys.max()) + 1)
    # 「完成」is right-aligned: grow left to ≥2.5 glyph squares + pad
    gh = max(14, ink.h())
    min_w = max(ink.w() + 12, int(gh * 2.6) + 12)
    x1 = min(w - 4, ink.x1 + 6)
    x0 = max(x_left, x1 - min_w)
    y0b = max(0, ink.y0 - 6)
    y1b = min(h, ink.y1 + 6)
    return BBox(x0, y0b, x1, y1b)


def find_cancel_box(bgr: np.ndarray, chrome_cy: int) -> BBox | None:
    """Red「取消」— union both glyphs, expand right to full two-char hit box.

    Color blobs often read as two tiny components (取 / 消); a half-glyph box
    is useless for review and for clicks — always cover the whole word.
    """
    h, w = bgr.shape[:2]
    y0 = max(0, chrome_cy - 36)
    y1 = min(h, chrome_cy + 36)
    x_right = int(w * 0.45)
    strip = bgr[y0:y1, 0:x_right]
    if strip.size == 0:
        return None
    hsv = cv2.cvtColor(strip, cv2.COLOR_BGR2HSV)
    mask = cv2.bitwise_or(
        cv2.inRange(hsv, (0, 40, 60), (20, 255, 255)),
        cv2.inRange(hsv, (155, 40, 60), (180, 255, 255)),
    )
    # Also accept strong R-channel red (WeChat toolbar anti-alias)
    b, g, r = cv2.split(strip)
    rgb_red = ((r.astype(np.int16) > g.astype(np.int16) + 12) & (r.astype(np.int16) > b.astype(np.int16) + 12) & (r > 90)).astype(np.uint8) * 255
    mask = cv2.bitwise_or(mask, rgb_red)
    k = cv2.getStructuringElement(cv2.MORPH_RECT, (28, 8))
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, k, iterations=2)
    ys, xs = np.where(mask > 0)
    if len(xs) == 0:
        ww = max(56, int(w * 0.12))
        return BBox(8, max(0, chrome_cy - 13), 8 + ww, chrome_cy + 13)
    ink = BBox(int(xs.min()), y0 + int(ys.min()), int(xs.max()) + 1, y0 + int(ys.max()) + 1)
    # 「取消」is left-aligned: grow right to ≥2.5 glyph squares + pad
    gh = max(14, ink.h())
    min_w = max(ink.w() + 12, int(gh * 2.6) + 12)
    x0 = max(4, ink.x0 - 6)
    x1 = min(x_right, x0 + min_w)
    # If still short of ink right edge, honor ink
    x1 = max(x1, ink.x1 + 6)
    y0b = max(0, ink.y0 - 6)
    y1b = min(h, ink.y1 + 6)
    return BBox(x0, y0b, x1, y1b)


def draw_labeled(img: np.ndarray, box: BBox, label: str, color: tuple[int, int, int]) -> None:
    cv2.rectangle(img, (box.x0, box.y0), (box.x1, box.y1), color, 2)
    tx, ty = box.x0, max(12, box.y0 - 3)
    cv2.putText(img, label, (tx, ty), cv2.FONT_HERSHEY_SIMPLEX, 0.45, color, 1, cv2.LINE_AA)


def main() -> None:
    import sys

    src = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SRC
    if not src.is_file():
        src = FALLBACK_SRC
    bgr = cv2.imread(str(src))
    if bgr is None:
        raise SystemExit(f"cannot read {src}")
    print(f"src={src}")
    h, w = bgr.shape[:2]
    body_top, body_bottom, chrome_cy, caps = estimate_keyboard_band(bgr)
    rows = cluster_key_rows(caps)

    labeled: list[tuple[str, BBox, str]] = []
    source = "keyboard_cap" if len(caps) >= 20 else "keyboard_geom"

    if len(caps) >= 20:
        for ri, row_boxes in enumerate(rows):
            if ri >= len(QWERTY_ROWS):
                break
            labels = QWERTY_ROWS[ri]
            snapped = snap_row_to_grid(row_boxes, len(labels), w)
            for i, ch in enumerate(labels):
                if i >= len(snapped):
                    break
                labeled.append((ch, snapped[i], source))
    else:
        # Uniform geometry fallback
        body_h = max(40, body_bottom - body_top)
        row_h = body_h / len(QWERTY_ROWS)
        for ri, row in enumerate(QWERTY_ROWS):
            y = body_top + row_h * (ri + 0.5)
            n = len(row)
            margin = w * 0.028
            usable = max(40.0, w - 2 * margin)
            cell = usable / 10.0
            row_w = cell * n
            x0 = (w - row_w) * 0.5
            half = float(np.clip(cell * 0.42, 8.0, 22.0))
            for i, ch in enumerate(row):
                cx = x0 + cell * (i + 0.5)
                labeled.append(
                    (
                        ch,
                        BBox(
                            int(round(cx - half)),
                            int(round(y - half)),
                            int(round(cx + half)),
                            int(round(y + half)),
                        ),
                        "keyboard_geom",
                    )
                )

    cancel = find_cancel_box(bgr, chrome_cy)
    done = find_done_box(bgr, chrome_cy)
    backspace = find_backspace_box(bgr, body_top)

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    stem = "clean"

    # Panel 1: raw white caps
    raw = bgr.copy()
    for b in caps:
        cv2.rectangle(raw, (b.x0, b.y0), (b.x1, b.y1), (0, 165, 255), 1)
    cv2.line(raw, (0, body_top), (w, body_top), (255, 0, 255), 1)
    cv2.putText(
        raw,
        f"raw caps={len(caps)} rows={len(rows)} body_top={body_top}",
        (8, 24),
        cv2.FONT_HERSHEY_SIMPLEX,
        0.55,
        (0, 0, 255),
        1,
        cv2.LINE_AA,
    )
    raw_path = OUT_DIR / f"{stem}-raw-caps.png"
    cv2.imwrite(str(raw_path), raw)

    # Panel 2: labeled QWERTY + chrome
    audit = bgr.copy()
    for ch, box, _src in labeled:
        draw_labeled(audit, box, ch, (0, 200, 0))
        cv2.circle(audit, (box.cx(), box.cy()), 3, (0, 0, 255), -1)
    if cancel:
        draw_labeled(audit, cancel, "cancel", (0, 0, 255))
        cv2.circle(audit, (cancel.cx(), cancel.cy()), 3, (0, 0, 255), -1)
    if done:
        draw_labeled(audit, done, "done", (0, 180, 0))
        cv2.circle(audit, (done.cx(), done.cy()), 3, (0, 180, 0), -1)
    if backspace:
        draw_labeled(audit, backspace, "del", (0, 0, 255))
        cv2.circle(audit, (backspace.cx(), backspace.cy()), 3, (0, 0, 255), -1)
    cv2.putText(
        audit,
        f"keys={len(labeled)}/36 cancel_w={cancel.w() if cancel else 0} src={source}",
        (8, 24),
        cv2.FONT_HERSHEY_SIMPLEX,
        0.55,
        (0, 0, 255),
        1,
        cv2.LINE_AA,
    )
    audit_path = OUT_DIR / f"{stem}-labeled-keys.png"
    cv2.imwrite(str(audit_path), audit)

    # Crop keyboard region for easier review
    crop_y0 = max(0, min(body_top, chrome_cy) - 40)
    crop = audit[crop_y0:h].copy()
    crop_path = OUT_DIR / f"{stem}-keyboard-centers.png"
    cv2.imwrite(str(crop_path), crop)

    print(f"image={w}x{h}")
    print(f"band body_top={body_top} body_bottom={body_bottom} chrome_cy={chrome_cy}")
    print(f"caps={len(caps)} rows={[len(r) for r in rows]} source={source}")
    print(f"labeled={len(labeled)}")
    for ch, box, src in labeled:
        print(f"  {ch}: cx={box.cx()} cy={box.cy()} box=({box.x0},{box.y0})-({box.x1},{box.y1}) [{src}]")
    for name, box in (("cancel", cancel), ("done", done), ("del", backspace)):
        if box:
            print(f"  {name}: cx={box.cx()} cy={box.cy()} w={box.w()} h={box.h()} box=({box.x0},{box.y0})-({box.x1},{box.y1})")
    print(f"wrote {raw_path}")
    print(f"wrote {audit_path}")
    print(f"wrote {crop_path}")


if __name__ == "__main__":
    main()
