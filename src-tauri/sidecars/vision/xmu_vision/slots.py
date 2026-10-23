"""Structured slot grid: time rows × court columns."""

from __future__ import annotations

import re
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

import cv2
import numpy as np

from xmu_vision.anchors import Box, _mint_masks, hit_box
from xmu_vision.locate import Match, match_template_file, ocr_image


_TIME_RE = re.compile(r"(\d{1,2}):(\d{2})")


class SlotState(str, Enum):
    AVAILABLE = "available"
    LOCKED = "locked"
    UNAVAILABLE = "unavailable"


FallbackOrder = str  # "up_then_down" | "down_then_up"


@dataclass(frozen=True)
class SlotCandidate:
    start_time: str
    end_time: str
    court_index: int | None
    court_label: str | None
    state: SlotState
    click_x: int
    click_y: int
    evidence: tuple[str, ...]

    @property
    def available(self) -> bool:
        return self.state == SlotState.AVAILABLE

    def as_match(self) -> Match:
        label = f"{self.start_time}|c{self.court_index or '?'}|{self.state.value}"
        return Match(self.click_x, self.click_y, 1.0 if self.available else 0.0, label=label)


def _parse_time_label(text: str) -> str | None:
    m = _TIME_RE.search(text.replace(" ", ""))
    if not m:
        return None
    return f"{int(m.group(1)):02d}:{m.group(2)}"


def _time_labels(image_bgr: np.ndarray, *, y0: int, y1: int, x1_max: int) -> list[tuple[str, Box]]:
    out: list[tuple[str, Box]] = []
    for hit in ocr_image(image_bgr):
        if hit.score < 0.45:
            continue
        t = _parse_time_label(hit.text)
        if t is None:
            continue
        b = hit_box(hit)
        if b.cy < y0 or b.cy > y1 or b.cx > x1_max:
            continue
        out.append((t, b))
    best: dict[str, Box] = {}
    for t, b in out:
        if t not in best or b.cx < best[t].cx:
            best[t] = b
    return sorted(best.items(), key=lambda kv: kv[1].cy)


def _court_headers(image_bgr: np.ndarray, *, y0: int, y1_header: int, x0: int) -> list[tuple[str, Box]]:
    """Optional numeric court headers like 01/02 in the header band."""
    headers: list[tuple[str, Box]] = []
    for hit in ocr_image(image_bgr):
        if hit.score < 0.45:
            continue
        raw = hit.text.strip().replace(" ", "")
        if not re.fullmatch(r"0?\d{1,2}", raw):
            continue
        b = hit_box(hit)
        if b.cy < y0 or b.cy > y1_header or b.cx < x0:
            continue
        headers.append((raw.zfill(2) if len(raw) <= 2 else raw, b))
    headers.sort(key=lambda kv: kv[1].cx)
    return headers


def _mint_cell_boxes(image_bgr: np.ndarray, *, y0: int, y1: int, x0: int) -> list[Box]:
    h, w = image_bgr.shape[:2]
    roi = image_bgr[y0:y1, x0:]
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    mask = _mint_masks(hsv)
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (3, 3))
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel, iterations=1)
    contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    boxes: list[Box] = []
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
        boxes.append(Box(x0 + x, y0 + y, x0 + x + bw, y0 + y + bh))
    return boxes


def _default_lock_template() -> Path | None:
    p = Path(__file__).resolve().parents[1] / "assets" / "templates" / "slot_lock.png"
    return p if p.is_file() else None


def _lock_centers(
    image_bgr: np.ndarray,
    *,
    y0: int,
    y1: int,
    x0: int,
    template_path: Path | None = None,
) -> list[tuple[int, int]]:
    """Detect padlock icons in the grid ROI (template + dark-blob fallback)."""
    h, w = image_bgr.shape[:2]
    centers: list[tuple[int, int]] = []
    tpl = template_path or _default_lock_template()
    if tpl is not None:
        try:
            m = match_template_file(image_bgr, tpl, threshold=0.62)
        except Exception:  # noqa: BLE001
            m = None
        # match_template_file returns best only — also run multi via raw match
        if m is not None and y0 <= m.y <= y1 and m.x >= x0:
            centers.append((m.x, m.y))
        try:
            from xmu_vision.locate import match_template

            tpl_img = cv2.imread(str(tpl), cv2.IMREAD_COLOR)
            if tpl_img is not None:
                roi = image_bgr[y0:y1, x0:]
                # Sliding best-of: downsample search by matching once on full then
                # find secondary peaks via TM_CCOEFF_NORMED
                res = cv2.matchTemplate(roi, tpl_img, cv2.TM_CCOEFF_NORMED)
                loc = np.where(res >= 0.62)
                th, tw = tpl_img.shape[:2]
                for yy, xx in zip(*loc):
                    cx = x0 + int(xx + tw / 2)
                    cy = y0 + int(yy + th / 2)
                    if any(abs(cx - px) < 18 and abs(cy - py) < 16 for px, py in centers):
                        continue
                    centers.append((cx, cy))
                    if len(centers) >= 80:
                        break
        except Exception:  # noqa: BLE001
            pass

    # Dark padlock blob fallback on gray cells (low sat, dark center)
    roi = image_bgr[y0:y1, x0:]
    if roi.size == 0:
        return centers
    hsv = cv2.cvtColor(roi, cv2.COLOR_BGR2HSV)
    grayish = cv2.inRange(hsv, (0, 0, 140), (180, 55, 235))
    dark = cv2.inRange(hsv, (0, 0, 40), (180, 80, 130))
    mask = cv2.bitwise_and(grayish, dark)
    # Prefer small blobs (icon-sized)
    kernel = cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (3, 3))
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel, iterations=1)
    contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    min_a = (h * w) * 0.00002
    max_a = (h * w) * 0.0025
    for cnt in contours:
        area = cv2.contourArea(cnt)
        if area < min_a or area > max_a:
            continue
        x, y, bw, bh = cv2.boundingRect(cnt)
        if bw < 6 or bh < 8 or bw > 40 or bh > 44:
            continue
        ar = bw / max(bh, 1)
        if ar < 0.35 or ar > 1.4:
            continue
        cx = x0 + x + bw // 2
        cy = y0 + y + bh // 2
        if any(abs(cx - px) < 16 and abs(cy - py) < 14 for px, py in centers):
            continue
        centers.append((cx, cy))
    return centers


def _assign_time_and_col(
    cx: int,
    cy: int,
    *,
    times: list[tuple[str, Box]],
    col_centers: list[int],
    headers: list[tuple[str, Box]],
    gym_mode: bool,
    cell_h: int = 28,
) -> tuple[str, int | None, str | None]:
    start = ""
    if times:
        start, tb = min(times, key=lambda kv: abs(kv[1].cy - cy))
        if abs(tb.cy - cy) > max(28, cell_h):
            start = ""
    col_i: int | None = None
    label: str | None = None
    if not gym_mode and col_centers:
        col_i = min(range(len(col_centers)), key=lambda i: abs(col_centers[i] - cx))
        if headers and col_i < len(headers):
            label = headers[col_i][0]
        elif headers:
            hi = min(range(len(headers)), key=lambda i: abs(headers[i][1].cx - cx))
            label = headers[hi][0]
            col_i = hi
    return start, col_i, label


def build_slot_grid(
    image_bgr: np.ndarray,
    *,
    gym_mode: bool = False,
    lock_template: Path | None = None,
) -> list[SlotCandidate]:
    """Parse grid cells into SlotCandidate rows with available|locked|unavailable."""
    h, w = image_bgr.shape[:2]
    y0 = int(h * 0.12)
    y1 = int(h * 0.78)
    x0 = int(w * 0.10)
    times = _time_labels(image_bgr, y0=y0, y1=y1, x1_max=int(w * 0.28))
    headers = [] if gym_mode else _court_headers(image_bgr, y0=int(h * 0.08), y1_header=int(h * 0.22), x0=x0)
    mint = _mint_cell_boxes(image_bgr, y0=y0, y1=y1, x0=x0 if not gym_mode else int(w * 0.35))
    locks = [] if gym_mode else _lock_centers(image_bgr, y0=y0, y1=y1, x0=x0, template_path=lock_template)

    # Column centers from mint + locks + headers
    xs = sorted([c.cx for c in mint] + [lx for lx, _ in locks] + [b.cx for _, b in headers])
    col_centers: list[int] = []
    for x in xs:
        if not col_centers or abs(x - col_centers[-1]) > 36:
            col_centers.append(x)
        else:
            col_centers[-1] = (col_centers[-1] + x) // 2

    candidates: list[SlotCandidate] = []
    seen: set[tuple[str, int | None]] = set()

    for cell in mint:
        start, col_i, label = _assign_time_and_col(
            cell.cx, cell.cy, times=times, col_centers=col_centers, headers=headers, gym_mode=gym_mode, cell_h=cell.h
        )
        key = (start, col_i)
        if key in seen and start:
            continue
        seen.add(key)
        candidates.append(
            SlotCandidate(
                start_time=start,
                end_time="",
                court_index=col_i,
                court_label=label,
                state=SlotState.AVAILABLE,
                click_x=cell.cx,
                click_y=cell.cy,
                evidence=(f"mint@{cell.cx},{cell.cy}", f"time={start or '?'}", f"col={col_i}"),
            )
        )

    for lx, ly in locks:
        # Skip if already covered by a mint cell nearby (false lock)
        if any(abs(c.click_x - lx) < 22 and abs(c.click_y - ly) < 18 and c.available for c in candidates):
            continue
        start, col_i, label = _assign_time_and_col(
            lx, ly, times=times, col_centers=col_centers, headers=headers, gym_mode=gym_mode
        )
        key = (start, col_i)
        if key in seen and start:
            continue
        seen.add(key)
        candidates.append(
            SlotCandidate(
                start_time=start,
                end_time="",
                court_index=col_i,
                court_label=label,
                state=SlotState.LOCKED,
                click_x=lx,
                click_y=ly,
                evidence=(f"lock@{lx},{ly}", f"time={start or '?'}", f"col={col_i}"),
            )
        )

    return candidates


def visible_time_starts(image_bgr: np.ndarray) -> list[str]:
    h, w = image_bgr.shape[:2]
    times = _time_labels(image_bgr, y0=int(h * 0.12), y1=int(h * 0.78), x1_max=int(w * 0.28))
    return [t for t, _ in times]


def nearest_time_order(
    times: list[str],
    target: str,
    *,
    order: FallbackOrder = "down_then_up",
) -> list[str]:
    """Target first, then neighbors by distance; tie-break by up/down preference."""
    if not times:
        return [target] if target.strip() else []
    normed: list[str] = []
    for t in times:
        p = _parse_time_label(t) or t
        if p not in normed:
            normed.append(p)
    needle = _parse_time_label(target) or target.strip()
    if not needle:
        return list(normed)
    if needle not in normed:
        return [needle] + normed
    idx = normed.index(needle)
    out = [needle]
    max_d = max(idx, len(normed) - 1 - idx)
    up_first = order == "up_then_down"
    for dist in range(1, max_d + 1):
        earlier = idx - dist
        later = idx + dist
        if up_first:
            if earlier >= 0:
                out.append(normed[earlier])
            if later < len(normed):
                out.append(normed[later])
        else:
            if later < len(normed):
                out.append(normed[later])
            if earlier >= 0:
                out.append(normed[earlier])
    return out


def pick_slots_for_time(
    candidates: list[SlotCandidate],
    *,
    target_time: str,
    left_to_right: bool = True,
) -> list[SlotCandidate]:
    """Lock target time row; return available courts L→R (or single gym cell)."""
    needle = target_time.strip()
    if not needle:
        ordered = sorted(candidates, key=lambda c: (c.click_y, c.click_x if left_to_right else -c.click_x))
        return [c for c in ordered if c.available]

    parsed = _parse_time_label(needle) or needle
    row = [
        c
        for c in candidates
        if c.available
        and (c.start_time == parsed or c.start_time.endswith(parsed) or parsed.endswith(c.start_time))
    ]
    if not row:
        return []
    row.sort(key=lambda c: c.click_x if left_to_right else -c.click_x)
    return row


def apply_court_offset(ordered: list, offset: int) -> list:
    """Put index min(offset, len-1) first; then right neighbors; then left skipped."""
    if len(ordered) <= 1:
        return list(ordered)
    i = min(max(0, int(offset)), len(ordered) - 1)
    if i == 0:
        return list(ordered)
    return list(ordered[i:]) + list(ordered[:i])


def find_available_slot_candidates(
    image_bgr: np.ndarray,
    *,
    prefer_time: str = "",
    prefer_court: str = "",
    gym_mode: bool = False,
    allow_time_fallback: bool = False,
    time_fallback_order: FallbackOrder = "down_then_up",
    court_offset: int = 2,
) -> list[SlotCandidate]:
    grid = build_slot_grid(image_bgr, gym_mode=gym_mode)
    if not prefer_time.strip():
        base = sorted([c for c in grid if c.available], key=lambda c: (c.click_y, c.click_x))
        return apply_court_offset(base, court_offset)

    times = visible_time_starts(image_bgr)
    sequence = (
        nearest_time_order(times, prefer_time.strip(), order=time_fallback_order)
        if allow_time_fallback
        else [(_parse_time_label(prefer_time.strip()) or prefer_time.strip())]
    )

    for t in sequence:
        picked = pick_slots_for_time(grid, target_time=t)
        if not picked:
            continue
        if prefer_court.strip():
            prefer = prefer_court.strip().zfill(2)
            preferred = [c for c in picked if (c.court_label or "").zfill(2) == prefer]
            rest = [c for c in picked if c not in preferred]
            picked = preferred + rest
        return apply_court_offset(picked, court_offset)
    return []


def count_states(candidates: list[SlotCandidate]) -> dict[str, int]:
    out = {s.value: 0 for s in SlotState}
    for c in candidates:
        out[c.state.value] = out.get(c.state.value, 0) + 1
    return out
