"""Element-anchored vision helpers — never use raw screen-percent clicks.

All clicks should be derived from OCR boxes / color blobs found in the
*current* window screenshot, then offset relative to that element's size.
"""

from __future__ import annotations

from dataclasses import dataclass

import cv2
import numpy as np

from xmu_vision.locate import Match, OcrHit, find_text, ocr_image


@dataclass(frozen=True)
class Box:
    x0: int
    y0: int
    x1: int
    y1: int

    @property
    def cx(self) -> int:
        return (self.x0 + self.x1) // 2

    @property
    def cy(self) -> int:
        return (self.y0 + self.y1) // 2

    @property
    def w(self) -> int:
        return max(1, self.x1 - self.x0)

    @property
    def h(self) -> int:
        return max(1, self.y1 - self.y0)

    def as_match(self, *, score: float = 1.0, label: str = "") -> Match:
        return Match(self.cx, self.cy, score, label=label)


def hit_box(hit: OcrHit) -> Box:
    xs = [p[0] for p in hit.box]
    ys = [p[1] for p in hit.box]
    return Box(int(min(xs)), int(min(ys)), int(max(xs)), int(max(ys)))


def find_hit(
    image_bgr: np.ndarray,
    needle: str,
    *,
    min_score: float = 0.5,
    substring: bool = True,
) -> OcrHit | None:
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
    return best


def point_right_of(box: Box, *, gap_widths: float = 0.8, down_frac: float = 0.5) -> tuple[int, int]:
    """Click to the right of an element, offset in units of that element's width/height."""
    x = int(box.x1 + gap_widths * box.w)
    y = int(box.y0 + down_frac * box.h)
    return x, y


def point_below(box: Box, *, gap_heights: float = 0.6, across_frac: float = 0.5) -> tuple[int, int]:
    x = int(box.x0 + across_frac * box.w)
    y = int(box.y1 + gap_heights * box.h)
    return x, y


def find_captcha_row(image_bgr: np.ndarray) -> tuple[Box, Box, tuple[int, int]] | None:
    """Locate captcha image + input click using the「验证码」label as anchor.

    Layout (from screenshots):
        [验证码]  [captcha image]  [ _ _ _ _ input ]

    Returns (label_box, captcha_box, input_click_xy) or None.
    """
    label = find_hit(image_bgr, "验证码")
    if label is None:
        return None
    lb = hit_box(label)
    h, w = image_bgr.shape[:2]
    # Horizontal strip around the label row — a bit taller than the label
    y0 = max(0, lb.y0 - lb.h)
    y1 = min(h, lb.y1 + lb.h)
    x_start = min(w - 1, lb.x1 + 4)
    strip = image_bgr[y0:y1, x_start:w]
    if strip.size == 0:
        return None

    # Captcha is the first high-variance block right of the label (speckled PNG),
    # not the underscore input further right.
    gray = cv2.cvtColor(strip, cv2.COLOR_BGR2GRAY)
    th, tw = gray.shape[:2]
    win_w = max(70, min(int(lb.w * 2.8), 150))
    win_h = th
    best_score = -1.0
    best_x = 0
    step = max(2, win_w // 10)
    # Prefer the leftmost strong blob (true captcha sits next to label)
    search_right = min(tw - win_w, max(win_w * 2, int(tw * 0.55)))
    for xx in range(0, max(1, search_right), step):
        patch = gray[:, xx : xx + win_w]
        std = float(patch.std())
        mean = float(patch.mean())
        if mean < 120 or mean > 250:
            continue
        # Slight left bias so we don't jump to input noise
        score = std - xx * 0.02
        if score > best_score:
            best_score = score
            best_x = xx
    if best_score < 8.0:
        # Fallback: element-relative square after label
        cap = Box(
            lb.x1 + max(8, lb.w // 4),
            lb.y0 - lb.h // 3,
            lb.x1 + int(3.0 * lb.w),
            lb.y1 + lb.h // 3,
        )
    else:
        cap = Box(x_start + best_x, y0, x_start + best_x + win_w, y0 + win_h)

    # Input is further right of captcha, same row — offset by captcha width
    ix, iy = point_right_of(cap, gap_widths=0.55, down_frac=0.55)
    ix = min(w - 4, max(0, ix))
    iy = min(h - 4, max(0, iy))
    return lb, cap, (ix, iy)


def _mint_masks(hsv: np.ndarray) -> np.ndarray:
    """Union of pale grid mint (pingpong/badminton) + stronger pill mint (gym).

    Real multi-court cells are near HSV≈(80, 25, 245) — low saturation.
    Gym list pills are usually more saturated.
    """
    pale = cv2.inRange(hsv, (68, 10, 200), (98, 95, 255))
    strong = cv2.inRange(hsv, (35, 40, 140), (100, 255, 255))
    return cv2.bitwise_or(pale, strong)


def find_available_slot_cells(
    image_bgr: np.ndarray,
    *,
    prefer_time: str = "",
    prefer_court: str = "",
) -> list[Match]:
    """Find「可预约」mint cells via color (+ optional OCR「0/1」), near prefer_time.

    Multi-court grids (乒乓球/羽毛球): many columns, pale mint +「0/1」.
    Gym: usually a single column of stronger mint pills.
    """
    h, w = image_bgr.shape[:2]
    # Crop away bottom legend / button (~22%) and top chrome (~12%)
    y0 = int(h * 0.12)
    y1 = int(h * 0.78)
    x0 = int(w * 0.12)  # skip time-label column on multi-court grids
    roi = image_bgr[y0:y1, x0:]
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    mask = _mint_masks(hsv)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel, iterations=1)
    contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)

    cells: list[Match] = []
    min_area = (h * w) * 0.00025
    max_area = (h * w) * 0.05
    for cnt in contours:
        area = cv2.contourArea(cnt)
        if area < min_area or area > max_area:
            continue
        x, y, bw, bh = cv2.boundingRect(cnt)
        if bw < 14 or bh < 14:
            continue
        ar = bw / max(bh, 1)
        if ar < 0.35 or ar > 3.2:
            continue
        cx = x0 + x + bw // 2
        cy = y0 + y + bh // 2
        cells.append(Match(cx, cy, float(area), label=f"slot@{cx},{cy}"))

    # OCR fallback / reinforcement:「0/1」= bookable on multi-court grids
    try:
        for hit in ocr_image(image_bgr):
            t = hit.text.strip().replace(" ", "")
            if t not in ("0/1", "0／1") and not (t.startswith("0/") and len(t) <= 4):
                continue
            if hit.score < 0.45:
                continue
            hb = hit_box(hit)
            if hb.cy < y0 or hb.cy > y1 or hb.cx < x0:
                continue
            # Dedup against nearby color cells
            if any(abs(m.x - hb.cx) < 28 and abs(m.y - hb.cy) < 22 for m in cells):
                continue
            cells.append(Match(hb.cx, hb.cy, float(hit.score) * 1000.0, label=f"ocr0/1@{hb.cx},{hb.cy}"))
    except RuntimeError:
        pass

    if not cells:
        return []

    if prefer_court.strip():
        court_hit = find_hit(image_bgr, prefer_court.strip())
        if court_hit is not None:
            cb = hit_box(court_hit)

            def court_score(m: Match) -> float:
                return abs(m.x - cb.cx) * 2.0 + abs(m.y - cb.cy) * 0.15

            cells = sorted(cells, key=court_score)

    if prefer_time.strip():
        time_hit = find_hit(image_bgr, prefer_time.strip())
        if time_hit is not None:
            tb = hit_box(time_hit)

            def score(m: Match) -> float:
                # Prefer cells to the right of / near the time label row
                dx = m.x - tb.x1
                dy = abs(m.y - tb.cy)
                if dx < -10:
                    return 1e12
                return dy * 4.0 + abs(dx) * 0.15

            cells = sorted(cells, key=score)
            return cells

    # Prefer evening (lower rows) then left-to-right among bookable courts
    cells.sort(key=lambda m: (m.y, m.x))
    return cells


def slot_looks_selected(image_bgr: np.ndarray) -> bool:
    """True when bottom CTA left「请先选择场次」(selection succeeded)."""
    if find_hit(image_bgr, "确认预约") or find_hit(image_bgr, "确定预约"):
        return True
    if find_hit(image_bgr, "请先选择场次"):
        return False
    return False


def find_confirm_button(image_bgr: np.ndarray) -> Match | None:
    """Bottom booking CTA — never the top nav title「确认预约」.

    - 选择场次页底部:「确认预约」
    - 验证码确认页底部:「确定预约」
    """
    h, w = image_bgr.shape[:2]
    prefer = ("确定预约", "确认预约", "提交预约")
    hits: list[Match] = []
    for hit in ocr_image(image_bgr):
        t = hit.text.strip()
        for label in prefer:
            if t == label or label in t:
                if hit.y < h * 0.55:
                    continue  # skip top title chrome
                hits.append(Match(hit.x, hit.y, hit.score, label=label))
                break
    if hits:
        # Prefer 确定预约, then lowest on screen
        def rank(m: Match) -> tuple:
            pri = 0 if m.label == "确定预约" else (1 if m.label == "确认预约" else 2)
            return (pri, -m.y)

        return sorted(hits, key=rank)[0]

    # Color fallback: full-width dark-blue bar in bottom 25%
    y0 = int(h * 0.75)
    roi = image_bgr[y0:h, :]
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    mask = cv2.inRange(hsv, (95, 40, 40), (130, 255, 200))
    cnts, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    best = None
    best_a = 0.0
    for c in cnts:
        a = float(cv2.contourArea(c))
        x, y, bw, bh = cv2.boundingRect(c)
        if a < (w * h) * 0.01 or bw < w * 0.45 or bh < 28:
            continue
        if a > best_a:
            best_a = a
            best = Match(x + bw // 2, y0 + y + bh // 2, a, label="cta_blue")
    if best is not None:
        return best
    if find_text(image_bgr, "请先选择场次"):
        return None
    return None


def captcha_toast_kind(image_bgr: np.ndarray) -> str | None:
    """Return toast kind on confirm page, or None.

    - ``need_input``:「请输入验证码」— clicked 确定预约 while empty
    - ``wrong``:「验证码错误」/「验证错误」/过期
    """
    if find_hit(image_bgr, "请输入验证码") or find_hit(image_bgr, "请输入验证"):
        return "need_input"
    if (
        find_hit(image_bgr, "验证码错误")
        or find_hit(image_bgr, "验证错误")
        or find_hit(image_bgr, "验证码已过期")
    ):
        return "wrong"
    return None


def captcha_error_toast(image_bgr: np.ndarray) -> bool:
    """True when any captcha-related toast is showing (wrong or need-input)."""
    return captcha_toast_kind(image_bgr) is not None


def keyboard_done(image_bgr: np.ndarray) -> Match | None:
    """Soft-keyboard toolbar green「完成」(top-right of keyboard panel)."""
    m = find_text(image_bgr, "完成")
    if m:
        return m
    # RapidOCR often misses green「完成」;「取消」on the same toolbar is reliable.
    cancel = find_hit(image_bgr, "取消")
    if cancel is None:
        return None
    cb = hit_box(cancel)
    h, w = image_bgr.shape[:2]
    x = min(w - 8, max(cb.cx + int(w * 0.55), w - int(cb.w * 3.5)))
    y0 = max(0, cb.y0 - cb.h)
    y1 = min(h, cb.y1 + cb.h * 2)
    strip = image_bgr[y0:y1, int(w * 0.55) : w]
    if strip.size:
        hsv = cv2.cvtColor(strip, cv2.COLOR_BGR2HSV)
        mask = cv2.inRange(hsv, (35, 60, 80), (95, 255, 255))
        cnts, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
        best = None
        best_a = 0.0
        for c in cnts:
            a = float(cv2.contourArea(c))
            if a >= 40 and a > best_a:
                best_a = a
                x2, y2, bw, bh = cv2.boundingRect(c)
                best = Match(
                    int(w * 0.55) + x2 + bw // 2,
                    y0 + y2 + bh // 2,
                    a,
                    label="keyboard_done_green",
                )
        if best is not None:
            return best
    return Match(x, cb.cy, 0.5, label="keyboard_done_vs_cancel")


def keyboard_backspace(image_bgr: np.ndarray) -> Match | None:
    """Red delete key at bottom-right of the soft keyboard."""
    cancel = find_hit(image_bgr, "取消")
    h, w = image_bgr.shape[:2]
    # Keyboard occupies lower ~40% when open
    y0 = int(h * 0.55)
    if cancel is not None:
        y0 = max(y0, hit_box(cancel).y0)
    roi = image_bgr[y0:h, int(w * 0.55) : w]
    if roi.size == 0:
        return None
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    # Saturated red (wraps around hue 0)
    mask1 = cv2.inRange(hsv, (0, 80, 80), (12, 255, 255))
    mask2 = cv2.inRange(hsv, (165, 80, 80), (180, 255, 255))
    mask = cv2.bitwise_or(mask1, mask2)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, kernel, iterations=2)
    cnts, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    best = None
    best_score = -1.0
    for c in cnts:
        a = float(cv2.contourArea(c))
        if a < 80:
            continue
        x, y, bw, bh = cv2.boundingRect(c)
        # Prefer large bottom-right blob
        cx = int(w * 0.55) + x + bw // 2
        cy = y0 + y + bh // 2
        score = a + cx * 0.5 + cy * 0.3
        if score > best_score:
            best_score = score
            best = Match(cx, cy, a, label="keyboard_backspace")
    return best


def notice_present(image_bgr: np.ndarray) -> bool:
    return bool(
        find_hit(image_bgr, "预约须知")
        or find_hit(image_bgr, "我知道了")
        or find_hit(image_bgr, "须知")
    )
