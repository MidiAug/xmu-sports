"""Stable WeChat PC nav: Discover (icon) → 小程序 → MP search → first result.

Sidebar「发现」has NO text — only a compass icon. OCR will fail.
Stable approach (industry RPA practice):
  1) Lock to WeChat window rect
  2) Restrict ROI to left sidebar strip
  3) Multi-scale template match (DPI) + WeChat-green circle fallback
  4) Never use absolute screen percentages for the primary path
"""

from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np

from xmu_vision.locate import Match, find_text, load_template, match_template, ocr_image


def sidebar_roi(image_bgr: np.ndarray, *, frac: float = 0.08) -> tuple[np.ndarray, int]:
    """Return (crop, x_offset). Sidebar is a thin strip on the left of the window."""
    h, w = image_bgr.shape[:2]
    sw = max(48, min(int(w * frac), 120))
    return image_bgr[:, :sw].copy(), 0


def match_template_multiscale(
    haystack_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    *,
    threshold: float = 0.72,
    scales: tuple[float, ...] = (0.75, 0.85, 0.95, 1.0, 1.1, 1.25, 1.4),
) -> Match | None:
    """Multi-scale TM_CCOEFF_NORMED — handles DPI / window zoom drift."""
    if haystack_bgr.size == 0 or needle_bgr.size == 0:
        return None
    best: Match | None = None
    nh, nw = needle_bgr.shape[:2]
    for s in scales:
        tw, th = max(8, int(nw * s)), max(8, int(nh * s))
        if th >= haystack_bgr.shape[0] or tw >= haystack_bgr.shape[1]:
            continue
        scaled = cv2.resize(needle_bgr, (tw, th), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_CUBIC)
        m = match_template(haystack_bgr, scaled, threshold=threshold)
        if m is None:
            continue
        if best is None or m.score > best.score:
            best = Match(m.x, m.y, m.score, label=f"scale={s:.2f}")
    return best


def _wechat_green_circles(sidebar_bgr: np.ndarray) -> list[tuple[int, int, int, int]]:
    """Return list of (cx, cy, w, h) for WeChat-green circular icons in sidebar."""
    hsv = cv2.cvtColor(sidebar_bgr, cv2.COLOR_BGR2HSV)
    mask = cv2.inRange(hsv, (40, 70, 70), (95, 255, 255))
    # Ignore annotation red if present in training crops
    red = cv2.bitwise_or(
        cv2.inRange(hsv, (0, 120, 80), (12, 255, 255)),
        cv2.inRange(hsv, (160, 120, 80), (180, 255, 255)),
    )
    mask = cv2.bitwise_and(mask, cv2.bitwise_not(red))
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3))
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel, iterations=1)
    cnts, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    h, w = sidebar_bgr.shape[:2]
    out: list[tuple[int, int, int, int]] = []
    for c in cnts:
        x, y, bw, bh = cv2.boundingRect(c)
        area = bw * bh
        if area < 60 or area > (h * w) * 0.15:
            continue
        if abs(bw - bh) > max(bw, bh) * 0.5:
            continue
        out.append((x + bw // 2, y + bh // 2, bw, bh))
    out.sort(key=lambda t: t[1])
    return out


def find_discover_icon(
    image_bgr: np.ndarray,
    *,
    template_dir: Path | None = None,
) -> Match | None:
    """Locate「发现」compass icon inside the left sidebar only."""
    side, _x0 = sidebar_roi(image_bgr)
    h, _w = image_bgr.shape[:2]

    # 1) Multi-scale template in sidebar
    if template_dir:
        tpl_path = template_dir / "discover_icon.png"
        if tpl_path.is_file():
            needle = load_template(tpl_path)
            m = match_template_multiscale(side, needle, threshold=0.70)
            if m is not None:
                return Match(m.x, m.y, m.score, label="discover:template")

    # 2) Fallback: green circular icons in sidebar mid band
    circles = _wechat_green_circles(side)
    # Prefer icons in middle of sidebar height (skip avatar top / menu bottom)
    mid = [c for c in circles if 0.18 * h < c[1] < 0.55 * h]
    if not mid and circles:
        mid = circles
    if not mid:
        # 3) Last fallback: grayish compass — edge density blobs in sidebar mid
        gray = cv2.cvtColor(side, cv2.COLOR_BGR2GRAY)
        edges = cv2.Canny(gray, 50, 150)
        y0, y1 = int(0.18 * h), int(0.55 * h)
        band = edges[y0:y1, :]
        # find densest local window ~ sidebar icon size
        win = max(18, min(36, side.shape[1] - 2))
        best = (-1.0, 0, 0)
        for yy in range(0, max(1, band.shape[0] - win), 4):
            for xx in range(0, max(1, band.shape[1] - win), 2):
                score = float(band[yy : yy + win, xx : xx + win].mean())
                if score > best[0]:
                    best = (score, xx + win // 2, y0 + yy + win // 2)
        if best[0] > 8.0:
            return Match(best[1], best[2], best[0] / 50.0, label="discover:edges")
        return None

    # When Discover is selected it is green; often the lowest green in the mid band
    # (chat may also be green when selected — pick by ordinal: typically 3rd/4th icon)
    # Heuristic: among mid greens, prefer the one closest to 32–40% window height
    target_y = int(0.42 * h)  # discover sits below chat/contacts/cube; chat-green is higher
    cx, cy, _bw, _bh = min(mid, key=lambda t: abs(t[1] - target_y))
    # Reject chat-tab green (too high in sidebar)
    if cy < 0.22 * h:
        lower = [c for c in mid if c[1] >= 0.25 * h]
        if lower:
            cx, cy, _bw, _bh = min(lower, key=lambda t: abs(t[1] - target_y))
        else:
            return None
    return Match(cx, cy, 0.85, label="discover:green")


def find_mp_search_icon(
    image_bgr: np.ndarray,
    *,
    template_dir: Path | None = None,
) -> Match | None:
    """Magnifier in the mini-program panel header (top-right of content), not chat search."""
    h, w = image_bgr.shape[:2]
    # Right content header band — avoid left chat list
    x0 = int(w * 0.45)
    y0, y1 = int(h * 0.0), int(h * 0.18)
    roi = image_bgr[y0:y1, x0:w]
    if template_dir:
        tpl = template_dir / "mp_search_icon.png"
        if tpl.is_file():
            m = match_template_multiscale(roi, load_template(tpl), threshold=0.65)
            if m is not None:
                return Match(x0 + m.x, y0 + m.y, m.score, label="mp_search:template")

    # Prefer OCR「小程序」title then click right-of relative to that row
    title = None
    for hit in ocr_image(roi):
        if "小程序" in hit.text and hit.score >= 0.45:
            title = hit
            break
    if title is not None:
        # Magnifier sits near the right edge of the same header row
        return Match(w - int(0.04 * w), y0 + title.y, 0.7, label="mp_search:right_of_title")

    # Edge blob in top-right corner of ROI
    gray = cv2.cvtColor(roi, cv2.COLOR_BGR2GRAY)
    edges = cv2.Canny(gray, 60, 160)
    rh, rw = edges.shape[:2]
    win = max(16, min(40, rh - 2, rw // 8))
    # Search only the right quarter of the header
    xx0 = int(rw * 0.7)
    best = (-1.0, 0, 0)
    for yy in range(0, max(1, rh - win), 3):
        for xx in range(xx0, max(xx0 + 1, rw - win), 3):
            score = float(edges[yy : yy + win, xx : xx + win].mean())
            if score > best[0]:
                best = (score, xx + win // 2, yy + win // 2)
    if best[0] > 6.0:
        return Match(x0 + best[1], y0 + best[2], best[0] / 50.0, label="mp_search:edges")
    return None


def find_first_app_result(image_bgr: np.ndarray, app_name: str = "厦大体育") -> Match | None:
    """Click the first「厦大体育」under 使用过的小程序 / search dropdown."""
    hits = [h for h in ocr_image(image_bgr) if app_name in h.text and h.score >= 0.4]
    if not hits:
        m = find_text(image_bgr, app_name)
        return m
    # Prefer hits below a「使用过的小程序」header if present
    header = None
    for h in ocr_image(image_bgr):
        if "使用过" in h.text or "用过的小程序" in h.text:
            header = h
            break
    if header is not None:
        below = [h for h in hits if h.y >= header.y - 5]
        if below:
            below.sort(key=lambda h: (h.y, h.x))
            best = below[0]
            return Match(best.x, best.y, best.score, label=best.text)
    hits.sort(key=lambda h: (h.y, h.x))
    best = hits[0]
    return Match(best.x, best.y, best.score, label=best.text)
