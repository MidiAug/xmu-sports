"""Navigation helpers: pop_until with re-classify each layer."""

from __future__ import annotations

import time
from dataclasses import dataclass
from typing import Callable

import numpy as np

from xmu_vision import input_ops, locate, screen, ui_state, window
from xmu_vision.ui_state import Page, UiSnapshot


@dataclass
class PopResult:
    ok: bool
    page: Page
    pops: int
    message: str
    snapshots: list[UiSnapshot]


def _find_back(image_bgr: np.ndarray) -> tuple[int, int] | None:
    """Mini-program back chevron — prefer OCR「返回」, else top-left chrome click."""
    hit = locate.find_text(image_bgr, "返回")
    if hit and hit.y < image_bgr.shape[0] * 0.2:
        return hit.x, hit.y
    # PC WeChat mini-program: back arrow in title bar left
    h, w = image_bgr.shape[:2]
    return int(w * 0.04), int(h * 0.06)


def pop_until(
    win: window.WindowInfo,
    target: Page,
    *,
    max_pops: int = 8,
    settle_s: float = 1.2,
    dry_run: bool = False,
    grab: Callable[[window.WindowInfo], np.ndarray] | None = None,
) -> PopResult:
    """Click back until body fingerprint matches ``target`` or budget exhausted.

    Re-classifies after every pop. Stops on UNKNOWN, repeated identical page, or max_pops.
    """
    grab_fn = grab or screen.grab_window
    snaps: list[UiSnapshot] = []
    prev_page: Page | None = None
    stagnant = 0

    for i in range(max_pops + 1):
        img = grab_fn(win)
        snap = ui_state.classify(img)
        snaps.append(snap)
        if snap.page == target:
            return PopResult(True, snap.page, i, f"reached {target.value}", snaps)
        if snap.page == Page.UNKNOWN:
            return PopResult(False, snap.page, i, "unknown page — stop", snaps)
        if prev_page is not None and snap.page == prev_page:
            stagnant += 1
            if stagnant >= 2:
                return PopResult(False, snap.page, i, f"no change after back ({snap.page.value})", snaps)
        else:
            stagnant = 0
        prev_page = snap.page
        if i >= max_pops:
            break
        back = _find_back(img)
        if back is None:
            return PopResult(False, snap.page, i, "no back control", snaps)
        if dry_run:
            return PopResult(False, snap.page, i, f"dry-run would pop from {snap.page.value}", snaps)
        window.activate(win)
        input_ops.click_in_window(win, back[0], back[1])
        time.sleep(settle_s)
        win = window.refresh(win)

    last = snaps[-1].page if snaps else Page.UNKNOWN
    return PopResult(False, last, len(snaps) - 1, f"budget exhausted; last={last.value}", snaps)
