"""Unit tests — no live WeChat required."""

from __future__ import annotations

import sys
from datetime import date
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from xmu_vision.anchors import Box, find_available_slot_cells, point_right_of  # noqa: E402
from xmu_vision.dates import date_ocr_needles, target_day  # noqa: E402
from xmu_vision.flow.open_booking import STEP_ORDER, Step  # noqa: E402
from xmu_vision.ui_state import CaptchaPhase, Page, can_submit_order  # noqa: E402
from xmu_vision.ui_state import UiSnapshot  # noqa: E402
from xmu_vision.wechat_nav import match_template_multiscale  # noqa: E402
from xmu_vision.locate import match_template  # noqa: E402


def test_multiscale_finds_scaled_patch():
    canvas = np.zeros((200, 200, 3), dtype=np.uint8)
    canvas[:] = (30, 30, 30)
    patch = np.zeros((40, 40, 3), dtype=np.uint8)
    for y in range(40):
        for x in range(40):
            patch[y, x] = ((x * 9 + y * 5) % 255, (x * 4) % 255, (y * 7) % 255)
    big = cv2.resize(patch, (48, 48))
    canvas[70:118, 70:118] = big
    m = match_template_multiscale(canvas, patch, threshold=0.7)
    assert m is not None
    assert abs(m.x - 94) < 12


def test_target_day_tomorrow():
    base = date(2026, 9, 17)
    assert target_day(1, today=base) == date(2026, 9, 18)


def test_date_ocr_needles_tomorrow():
    base = date(2026, 9, 17)
    needles = date_ocr_needles(1, today=base)
    assert needles[0] == "明天"
    assert "周五" in needles


def test_step_order_tail():
    assert STEP_ORDER.index(Step.SELECT_DATE) < STEP_ORDER.index(Step.SELECT_SLOT)
    assert STEP_ORDER.index(Step.SELECT_SLOT) < STEP_ORDER.index(Step.CONFIRM_SLOT)
    assert STEP_ORDER.index(Step.CONFIRM_SLOT) < STEP_ORDER.index(Step.FILL_CAPTCHA)


def test_point_right_of_scales_with_element():
    small = Box(10, 10, 20, 20)  # w=10
    big = Box(10, 10, 50, 50)  # w=40
    sx, _ = point_right_of(small, gap_widths=1.0)
    bx, _ = point_right_of(big, gap_widths=1.0)
    assert bx - 50 == (sx - 20) * 4  # offset scales with width


def test_find_available_slot_cells_mint():
    canvas = np.zeros((400, 300, 3), dtype=np.uint8)
    canvas[:] = (240, 240, 240)
    # Pale multi-court mint ≈ HSV(80,24,246) → BGR(~238,246,223)
    canvas[120:160, 100:160] = (238, 246, 223)
    cells = find_available_slot_cells(canvas)
    assert cells, "expected at least one mint cell"
    assert abs(cells[0].x - 130) < 15
    assert abs(cells[0].y - 140) < 20


def test_match_template_finds_patch():
    canvas = np.zeros((200, 300, 3), dtype=np.uint8)
    canvas[:] = (40, 40, 40)
    patch = np.zeros((40, 50, 3), dtype=np.uint8)
    for y in range(40):
        for x in range(50):
            patch[y, x] = ((x * 7 + y * 13) % 255, (x * 3) % 255, (y * 11) % 255)
    canvas[80:120, 100:150] = patch
    m = match_template(canvas, patch, threshold=0.9)
    assert m is not None
    assert abs(m.x - 125) <= 2


def test_can_submit_order_requires_full_idle():
    empty = UiSnapshot(Page.CONFIRM_ORDER, "确认预约", captcha=CaptchaPhase.IDLE_EMPTY)
    assert not can_submit_order(empty)
    need = UiSnapshot(
        Page.CONFIRM_ORDER,
        "确认预约",
        captcha=CaptchaPhase.TOAST_NEED_INPUT,
        toast_need_input=True,
    )
    assert not can_submit_order(need)
    full = UiSnapshot(
        Page.CONFIRM_ORDER,
        "确认预约",
        captcha=CaptchaPhase.IDLE_FULL,
        keyboard_open=False,
        input_chars="ABCD",
    )
    assert can_submit_order(full)
    kb = UiSnapshot(
        Page.CONFIRM_ORDER,
        "确认预约",
        captcha=CaptchaPhase.KEYBOARD_OPEN_FULL,
        keyboard_open=True,
        input_chars="ABCD",
    )
    assert not can_submit_order(kb)


def test_resume_step_from_pages():
    from xmu_vision.ui_state import Page, SlotCta, UiSnapshot, resume_step_name

    assert resume_step_name(UiSnapshot(Page.HOME, "首页")) == "venue_booking"
    assert resume_step_name(UiSnapshot(Page.VENUE_DETAIL, "场馆详情")) == "select_date"
    assert resume_step_name(UiSnapshot(Page.CONFIRM_ORDER, "确认预约")) == "fill_captcha"
    ready = UiSnapshot(Page.SELECT_SLOT, "选择场次", slot_cta=SlotCta.READY)
    assert resume_step_name(ready) == "confirm_slot"
    need = UiSnapshot(Page.SELECT_SLOT, "选择场次", slot_cta=SlotCta.NEED_SELECT)
    assert resume_step_name(need) == "select_slot"


if __name__ == "__main__":
    test_multiscale_finds_scaled_patch()
    test_target_day_tomorrow()
    test_date_ocr_needles_tomorrow()
    test_step_order_tail()
    test_point_right_of_scales_with_element()
    test_find_available_slot_cells_mint()
    test_match_template_finds_patch()
    test_can_submit_order_requires_full_idle()
    test_resume_step_from_pages()
    print("ok")
