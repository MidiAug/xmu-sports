"""Classify regression — body fingerprints; title is never sole identity."""

from __future__ import annotations

import sys
from pathlib import Path
from unittest.mock import patch

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from xmu_vision.locate import Match, OcrHit  # noqa: E402
from xmu_vision.ui_state import Page, classify, has_my_reservations_list, has_sport_icon_grid  # noqa: E402
from xmu_vision.ui_state import keyboard_is_open  # noqa: E402


def _hit(text: str, x: int, y: int, score: float = 0.9) -> OcrHit:
    return OcrHit(text, x, y, score, ((x - 10, y - 5), (x + 10, y - 5), (x + 10, y + 5), (x - 10, y + 5)))


def test_sport_grid_beats_my_reservations_link_and_stale_title():
    """顶栏/标题可写「场馆列表」，右上角有「我的预约」链接 — 仍应是运动入口。"""
    img = np.zeros((800, 400, 3), dtype=np.uint8)
    hits = [
        _hit("场馆列表", 200, 40),  # stale chrome title
        _hit("我的预约", 340, 90),  # header link — must NOT win
        _hit("乒乓球", 80, 220),
        _hit("羽毛球", 160, 220),
        _hit("篮球", 240, 220),
        _hit("所有场馆", 80, 480),
    ]

    def fake_ocr(_img):
        return hits

    def fake_find(image, needle, **kwargs):
        for h in hits:
            if needle in h.text:
                return Match(h.x, h.y, h.score, label=h.text)
        return None

    with patch("xmu_vision.ui_state.ocr_image", fake_ocr), patch(
        "xmu_vision.locate.find_text", fake_find
    ), patch("xmu_vision.ui_state.locate.find_text", fake_find):
        assert has_sport_icon_grid(img)
        assert not has_my_reservations_list(img)
        snap = classify(img)
        assert snap.page == Page.VENUE_SPORT_LIST, snap
        assert "grid:sport_icons" in ",".join(snap.evidence) or "乒乓球" in ",".join(snap.evidence)


def test_my_reservations_requires_list_body():
    img = np.zeros((800, 400, 3), dtype=np.uint8)
    # Only the link — must NOT classify as my_reservations
    link_only = [_hit("我的预约", 340, 90), _hit("乒乓球", 80, 220), _hit("羽毛球", 160, 220)]

    def fake_ocr_link(_):
        return link_only

    def fake_find_link(image, needle, **kwargs):
        for h in link_only:
            if needle in h.text:
                return Match(h.x, h.y, h.score, label=h.text)
        return None

    with patch("xmu_vision.ui_state.ocr_image", fake_ocr_link), patch(
        "xmu_vision.ui_state.locate.find_text", fake_find_link
    ):
        snap = classify(img)
        assert snap.page != Page.MY_RESERVATIONS

    list_hits = [
        _hit("我的预约", 200, 80),
        _hit("已预约", 100, 200),
        _hit("已经到底", 200, 700),
    ]

    def fake_ocr_list(_):
        return list_hits

    def fake_find_list(image, needle, **kwargs):
        for h in list_hits:
            if needle in h.text:
                return Match(h.x, h.y, h.score, label=h.text)
        return None

    with patch("xmu_vision.ui_state.ocr_image", fake_ocr_list), patch(
        "xmu_vision.ui_state.locate.find_text", fake_find_list
    ):
        snap = classify(img)
        assert snap.page == Page.MY_RESERVATIONS


def test_venue_list_needs_rows_without_sport_grid():
    img = np.zeros((800, 400, 3), dtype=np.uint8)
    hits = [
        _hit("场馆列表", 200, 40),  # title only — insufficient if we also lack rows
        _hit("场馆列表", 200, 160),  # body
        _hit("翔安校区乒乓球室", 180, 300),
        _hit("思明校区", 100, 300),
        _hit("球馆", 100, 400),
    ]

    def fake_ocr(_):
        return hits

    def fake_find(image, needle, **kwargs):
        for h in hits:
            if needle in h.text:
                return Match(h.x, h.y, h.score, label=h.text)
        return None

    with patch("xmu_vision.ui_state.ocr_image", fake_ocr), patch(
        "xmu_vision.ui_state.locate.find_text", fake_find
    ):
        snap = classify(img)
        assert snap.page == Page.VENUE_LIST


def test_keyboard_ignores_mid_screen_cancel():
    img = np.zeros((800, 400, 3), dtype=np.uint8)
    # Modal-like cancel in middle — must not open keyboard
    hits = [_hit("取消", 200, 400), _hit("确定", 280, 400)]

    def fake_ocr(_):
        return hits

    def fake_find(image, needle, **kwargs):
        # locate.find_text on ROI: only lower half would include y>=416 for h=800
        h = image.shape[0]
        # When called on full image via anchors paths — patch lower-level
        for hit in hits:
            if needle in hit.text:
                # Simulate ROI crop: if image is shorter, coords are relative
                return Match(hit.x, hit.y if image.shape[0] == 800 else hit.y - 416, hit.score)
        return None

    with patch("xmu_vision.ui_state.locate.find_text", fake_find), patch(
        "xmu_vision.anchors.keyboard_backspace", return_value=None
    ), patch("xmu_vision.anchors.keyboard_done", return_value=None):
        # No digits in lower ROI → closed
        assert keyboard_is_open(img) is False


def test_fixture_sport_entrance_if_ocr_available():
    path = ROOT / "fixtures" / "title_stale_venue_list_but_sport_grid" / "shot.png"
    if not path.is_file():
        return
    try:
        from rapidocr_onnxruntime import RapidOCR  # noqa: F401
    except Exception:
        return
    img = cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_COLOR)
    assert img is not None
    snap = classify(img)
    assert snap.page == Page.VENUE_SPORT_LIST, (
        f"expected venue_sport_list got {snap.page} title={snap.title_hint!r} ev={snap.evidence}"
    )
    assert snap.page != Page.MY_RESERVATIONS


def test_pick_slots_time_locked():
    from xmu_vision.slots import SlotCandidate, SlotState, nearest_time_order, pick_slots_for_time

    cands = [
        SlotCandidate("19:30", "21:00", 0, "01", SlotState.AVAILABLE, 100, 200, ()),
        SlotCandidate("19:30", "21:00", 1, "02", SlotState.AVAILABLE, 200, 200, ()),
        SlotCandidate("18:00", "19:30", 0, "01", SlotState.AVAILABLE, 100, 100, ()),
        SlotCandidate("19:30", "21:00", 2, "03", SlotState.LOCKED, 300, 200, ()),
    ]
    row = pick_slots_for_time(cands, target_time="19:30")
    assert [c.court_label for c in row] == ["01", "02"]
    assert pick_slots_for_time(cands, target_time="20:00") == []

    times = ["09:00", "10:30", "12:00", "13:30"]
    assert nearest_time_order(times, "12:00", order="down_then_up") == [
        "12:00",
        "13:30",
        "10:30",
        "09:00",
    ]
    assert nearest_time_order(times, "12:00", order="up_then_down") == [
        "12:00",
        "10:30",
        "13:30",
        "09:00",
    ]


def test_apply_court_offset():
    from xmu_vision.slots import apply_court_offset

    assert apply_court_offset(["a", "b", "c", "d", "e"], 2) == ["c", "d", "e", "a", "b"]
    assert apply_court_offset(["a", "b"], 2) == ["b", "a"]
    assert apply_court_offset(["only"], 9) == ["only"]
    assert apply_court_offset([], 2) == []
    assert apply_court_offset(["a", "b", "c"], 0) == ["a", "b", "c"]

def test_observe_submit_outcomes():
    from xmu_vision.captcha_kb import SubmitOutcome, observe_submit_feedback

    class FakeWin:
        pass

    frames = [
        # toast wrong
        "wrong",
    ]
    state = {"i": 0}

    def grab(_w):
        img = np.zeros((600, 300, 3), dtype=np.uint8)
        return img

    with patch("xmu_vision.captcha_kb.anchors.captcha_toast_kind", return_value="wrong"), patch(
        "xmu_vision.captcha_kb.ui_state.classify",
        return_value=__import__("xmu_vision.ui_state", fromlist=["UiSnapshot"]).UiSnapshot(
            Page.CONFIRM_ORDER, "确认预约"
        ),
    ):
        r = observe_submit_feedback(FakeWin(), duration_s=0.3, interval_s=0.05, grab=grab)
        assert r.outcome == SubmitOutcome.CAPTCHA_WRONG


def test_contracts_roundtrip(tmp_path):
    from xmu_vision.contracts import BookingTask, EventSink, RunEvent

    t = BookingTask(
        task_id="t1",
        sport="乒乓球",
        venue="翔安校区乒乓球室",
        target_date="2026-09-19",
        fire_at="2026-09-18T07:01:00+08:00",
        target_time_start="19:30",
        target_time_end="21:00",
    )
    assert BookingTask.from_dict(t.to_dict()).sport == "乒乓球"
    sink = EventSink(tmp_path / "events.jsonl")
    sink.emit(RunEvent.now("t1", page_id="select_slot", action="click_slot", evidence=["mint"]))
    lines = (tmp_path / "events.jsonl").read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 1
    assert "select_slot" in lines[0]


def test_pop_until_reclassifies():
    from xmu_vision.nav import pop_until
    from xmu_vision.ui_state import UiSnapshot

    class FakeWin:
        pass

    pages = [Page.ORDER_DETAIL, Page.MY_RESERVATIONS, Page.VENUE_SPORT_LIST]
    idx = {"i": 0}

    def grab(_w):
        return np.zeros((100, 100, 3), dtype=np.uint8)

    def fake_classify(_img):
        i = min(idx["i"], len(pages) - 1)
        p = pages[i]
        idx["i"] += 1
        return UiSnapshot(p, "t")

    with patch("xmu_vision.nav.ui_state.classify", side_effect=fake_classify), patch(
        "xmu_vision.nav.input_ops.click_in_window"
    ), patch("xmu_vision.nav.window.activate"), patch(
        "xmu_vision.nav.window.refresh", side_effect=lambda w: w
    ), patch("xmu_vision.nav.time.sleep"):
        r = pop_until(FakeWin(), Page.VENUE_SPORT_LIST, max_pops=5, settle_s=0, grab=grab)
        assert r.ok
        assert r.page == Page.VENUE_SPORT_LIST
        assert r.pops >= 1


if __name__ == "__main__":
    test_sport_grid_beats_my_reservations_link_and_stale_title()
    test_my_reservations_requires_list_body()
    test_venue_list_needs_rows_without_sport_grid()
    test_keyboard_ignores_mid_screen_cancel()
    test_pick_slots_time_locked()
    test_apply_court_offset()
    test_observe_submit_outcomes()
    from pathlib import Path as P

    import tempfile

    with tempfile.TemporaryDirectory() as d:
        test_contracts_roundtrip(P(d))
    test_pop_until_reclassifies()
    test_fixture_sport_entrance_if_ocr_available()
    print("ok")
