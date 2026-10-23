"""Cancel an active reservation: 我的预约 → 已预约 → 取消预约 → 确定.

One action per step, brief wait, one check. No busy loops.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from xmu_vision import input_ops, locate, screen, ui_state, window
from xmu_vision.dpi import ensure_dpi_awareness
from xmu_vision.ui_state import Page


class CancelStep(str, Enum):
    BIND = "bind"
    OPEN_ACTIVE = "open_active"  # click 已预约
    TAP_CANCEL = "tap_cancel"  # click 取消预约
    CONFIRM = "confirm"  # modal 确定
    DONE = "done"


STEP_ORDER = [
    CancelStep.BIND,
    CancelStep.OPEN_ACTIVE,
    CancelStep.TAP_CANCEL,
    CancelStep.CONFIRM,
    CancelStep.DONE,
]


@dataclass
class CancelConfig:
    app_name: str = "厦大体育"
    venue_hint: str = "乒乓球"  # prefer active row containing this
    artifact_dir: Path = Path("artifacts/vision/cancel")
    dry_run: bool = False
    step_delay: float = 2.0


@dataclass
class CancelResult:
    ok: bool
    stopped_at: CancelStep
    message: str
    artifacts: list[Path] = field(default_factory=list)


class CancelBookingFlow:
    def __init__(self, cfg: CancelConfig) -> None:
        self.cfg = cfg
        self.artifacts: list[Path] = []
        self._win: window.WindowInfo | None = None

    def run(self) -> CancelResult:
        ensure_dpi_awareness()
        self.cfg.artifact_dir.mkdir(parents=True, exist_ok=True)
        for step in STEP_ORDER:
            if step == CancelStep.DONE:
                return CancelResult(True, CancelStep.DONE, "cancel finished", self.artifacts)
            try:
                self._run_step(step)
                self._log(f"after_{step.value}")
            except Exception as exc:  # noqa: BLE001
                self._snap(f"fail_{step.value}")
                self._log(f"fail_{step.value}")
                return CancelResult(False, step, str(exc), self.artifacts)
            time.sleep(self.cfg.step_delay)
        return CancelResult(True, CancelStep.DONE, "ok", self.artifacts)

    def _run_step(self, step: CancelStep) -> None:
        {
            CancelStep.BIND: self._step_bind,
            CancelStep.OPEN_ACTIVE: self._step_open_active,
            CancelStep.TAP_CANCEL: self._step_tap_cancel,
            CancelStep.CONFIRM: self._step_confirm,
        }[step]()

    def _active(self) -> window.WindowInfo:
        if self._win is None:
            raise RuntimeError("window not bound")
        self._win = window.refresh(self._win)
        return self._win

    def _grab(self):
        return screen.grab_window(self._active())

    def _snap(self, name: str):
        path = screen.save_bgr(self.cfg.artifact_dir / f"{name}.png", self._grab())
        self.artifacts.append(path)
        return path

    def _log(self, tag: str) -> None:
        snap = ui_state.classify(self._grab())
        print(f"[{tag}] {ui_state.snapshot_summary(snap)}")

    def _click(self, x: int, y: int, *, note: str = "") -> None:
        win = self._active()
        if self.cfg.dry_run:
            print(f"[dry-run] click ({x},{y}) {note}")
            return
        window.activate(win)
        input_ops.click_in_window(win, x, y)

    def _step_bind(self) -> None:
        mp = window.find_miniprogram(title_hint=self.cfg.app_name)
        if mp is None:
            raise RuntimeError("未找到小程序窗口")
        self._win = mp
        if not self.cfg.dry_run:
            window.activate(mp)
        img = self._grab()
        snap = ui_state.classify(img)
        print(f"[bind] {ui_state.snapshot_summary(snap)}")
        self._snap("01_bind")
        if snap.page == Page.ORDER_DETAIL:
            print("already on order detail — skip list")
            return
        if snap.page == Page.CANCEL_MODAL:
            print("cancel modal already open — skip to confirm")
            return
        if snap.page not in (Page.MY_RESERVATIONS, Page.UNKNOWN):
            # UNKNOWN still OK if 已预约 visible
            if not locate.find_text(img, "已预约"):
                raise RuntimeError(f"请先打开「我的预约」页 (got {snap.page.value})")

    def _step_open_active(self) -> None:
        img = self._grab()
        snap = ui_state.classify(img)
        if snap.page in (Page.ORDER_DETAIL, Page.CANCEL_MODAL):
            print("skip open_active — already past list")
            self._snap("02_skip_open")
            return

        # Prefer「已预约」near venue_hint (乒乓球)
        hits = [
            h
            for h in locate.ocr_image(img)
            if "已预约" in h.text and "取消" not in h.text
        ]
        if not hits:
            raise RuntimeError("列表中未找到「已预约」")

        hint = self.cfg.venue_hint.strip()
        best = min(hits, key=lambda h: h.y)
        if hint:
            venues = [h for h in locate.ocr_image(img) if hint in h.text]
            scored = []
            for h in hits:
                for v in venues:
                    if abs(v.y - h.y) < 80:
                        scored.append((abs(v.y - h.y), h))
            if scored:
                best = min(scored, key=lambda t: t[0])[1]

        print(f"[open_active] click 已预约 at ({best.x},{best.y})")
        self._click(best.x, best.y, note="已预约")
        time.sleep(self.cfg.step_delay)
        img2 = self._grab()
        snap2 = ui_state.classify(img2)
        self._snap("02_after_open")
        if snap2.page not in (Page.ORDER_DETAIL, Page.UNKNOWN) and not locate.find_text(
            img2, "取消预约"
        ):
            raise RuntimeError(f"点已预约后未进入详情 (got {snap2.page.value})")

    def _step_tap_cancel(self) -> None:
        img = self._grab()
        snap = ui_state.classify(img)
        if snap.page == Page.CANCEL_MODAL:
            print("skip tap_cancel — modal already open")
            self._snap("03_skip_cancel")
            return

        # Prefer top-right「取消预约」(header), else any「取消预约」
        cands = [h for h in locate.ocr_image(img) if h.text.strip() == "取消预约" or h.text.strip().endswith("取消预约")]
        if not cands:
            cands = [h for h in locate.ocr_image(img) if "取消预约" in h.text]
        if not cands:
            raise RuntimeError("详情页未找到「取消预约」")

        # Prefer highest on screen (header button)
        best = min(cands, key=lambda h: (h.y, -h.x))
        print(f"[tap_cancel] click 取消预约 at ({best.x},{best.y})")
        self._click(best.x, best.y, note="取消预约")
        time.sleep(self.cfg.step_delay)
        img2 = self._grab()
        snap2 = ui_state.classify(img2)
        self._snap("03_after_cancel_tap")
        if snap2.page != Page.CANCEL_MODAL and not (
            locate.find_text(img2, "确定要取消") or locate.find_text(img2, "你确定要取消")
        ):
            # Modal text OCR may vary — look for 提示 + 确定 button pair
            if not (locate.find_text(img2, "提示") and locate.find_text(img2, "确定")):
                raise RuntimeError(f"未出现取消确认弹窗 (got {snap2.page.value})")

    def _step_confirm(self) -> None:
        img = self._grab()
        # Modal「确定」— not bottom-page 取消, not 确定预约
        # Prefer solid button: text exactly「确定」, lower than title, left of「取消」
        ok_hits = [
            h
            for h in locate.ocr_image(img)
            if h.text.strip() == "确定" or h.text.strip() in ("确定",)
        ]
        ok_hits = [h for h in ok_hits if "预约" not in h.text]
        if not ok_hits:
            # substring
            ok_hits = [
                h
                for h in locate.ocr_image(img)
                if h.text.strip() == "确定"
                or (h.text.strip().endswith("确定") and "预约" not in h.text)
            ]
        if not ok_hits:
            raise RuntimeError("弹窗中未找到「确定」")

        cancel_btn = locate.find_text(img, "取消")
        # Pick 确定 that is near modal center (not tiny chrome)
        h_img = img.shape[0]
        mid = [h for h in ok_hits if h_img * 0.35 < h.y < h_img * 0.75]
        pool = mid or ok_hits
        if cancel_btn:
            # Prefer 确定 to the left of 取消 on same row
            same_row = [h for h in pool if abs(h.y - cancel_btn.y) < 40 and h.x < cancel_btn.x]
            if same_row:
                pool = same_row
        best = min(pool, key=lambda h: h.x)
        print(f"[confirm] click 确定 at ({best.x},{best.y})")
        self._click(best.x, best.y, note="弹窗确定")
        time.sleep(self.cfg.step_delay)
        img2 = self._grab()
        snap2 = ui_state.classify(img2)
        self._snap("04_after_confirm")
        print(f"[confirm result] {ui_state.snapshot_summary(snap2)}")
        # Success: back to list / detail shows 已取消 / modal gone
        if snap2.page == Page.CANCEL_MODAL:
            raise RuntimeError("点击确定后弹窗仍在")
