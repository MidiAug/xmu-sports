"""Flow: WeChat → 厦大体育 → 场馆预约 → 选日/场次 → 验证码 → 确定预约.

Vision rule: every click is anchored to OCR text / color blob in the *current*
window screenshot (see anchors.py). Never use raw screen-percentage coords.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path

from xmu_vision import anchors, captcha_kb, dates, input_ops, locate, screen, slots, ui_state, wechat_nav, window
from xmu_vision.contracts import EventSink, RunEvent, run_dir
from xmu_vision.dpi import ensure_dpi_awareness
from xmu_vision.ui_state import Page, SlotCta
from xmu_vision.ui_state import Overlay


class Step(str, Enum):
    ACTIVATE = "activate"
    DISCOVER = "discover"
    MINIPROGRAM = "miniprogram"
    SEARCH = "search"
    OPEN_APP = "open_app"
    VENUE_BOOKING = "venue_booking"
    DISMISS_NOTICE = "dismiss_notice"
    SELECT_SPORT = "select_sport"
    SELECT_VENUE = "select_venue"
    SELECT_DATE = "select_date"
    SELECT_SLOT = "select_slot"
    CONFIRM_SLOT = "confirm_slot"
    FILL_CAPTCHA = "fill_captcha"
    DONE = "done"


STEP_ORDER = [
    Step.ACTIVATE,
    Step.DISCOVER,
    Step.MINIPROGRAM,
    Step.SEARCH,
    Step.OPEN_APP,
    Step.VENUE_BOOKING,
    Step.DISMISS_NOTICE,
    Step.SELECT_SPORT,
    Step.SELECT_VENUE,
    Step.SELECT_DATE,
    Step.SELECT_SLOT,
    Step.CONFIRM_SLOT,
    Step.FILL_CAPTCHA,
    Step.DONE,
]


@dataclass
class FlowConfig:
    app_name: str = "厦大体育"
    sport: str = "乒乓球"
    venue: str = ""
    date_offset: int = 1
    # Prefer a time label near the slot (e.g. "19:30" / "10:30"); empty = first mint cell
    prefer_time: str = ""
    # Optional court header on multi-column grids (e.g. "02"); empty = leftmost bookable
    prefer_court: str = ""
    # Skip N leftmost available courts in the target row (0=leftmost). Default 2.
    court_offset: int = 2
    allow_time_fallback: bool = False
    time_fallback_order: str = "down_then_up"  # up_then_down | down_then_up
    captcha_retries: int = 5
    artifact_dir: Path = Path("artifacts/vision")
    dry_run: bool = False
    step_delay: float = 1.2
    template_dir: Path | None = None
    ocr: bool = True
    from_step: Step = Step.ACTIVATE
    until_step: Step = Step.FILL_CAPTCHA
    # If True (default): screenshot → classify page → jump to matching step
    auto_resume: bool = True
    # Structured run events (Tauri / morning replay)
    task_id: str = ""
    events_path: Path | None = None
    # After success, pop stack until 运动入口 (re-classify each layer)
    pop_to_sport_list: bool = False


@dataclass
class FlowResult:
    ok: bool
    stopped_at: Step
    message: str
    artifacts: list[Path] = field(default_factory=list)


class OpenBookingFlow:
    def __init__(self, cfg: FlowConfig) -> None:
        self.cfg = cfg
        self.artifacts: list[Path] = []
        self._win: window.WindowInfo | None = None
        self._sink: EventSink | None = None
        if cfg.task_id:
            ev = cfg.events_path
            if ev is None:
                ev = run_dir(cfg.task_id) / "events.jsonl"
            self._sink = EventSink(ev)

    def _emit(
        self,
        *,
        action: str | None = None,
        result: str | None = None,
        snap: ui_state.UiSnapshot | None = None,
    ) -> None:
        if self._sink is None or not self.cfg.task_id:
            return
        overlay = None
        page_id = None
        evidence: list[str] = []
        conf = None
        if snap is not None:
            page_id = snap.page.value
            evidence = list(snap.evidence)
            conf = snap.confidence
            if snap.overlay != Overlay.NONE:
                overlay = snap.overlay.value
        arts = [str(p) for p in self.artifacts[-3:]]
        self._sink.emit(
            RunEvent.now(
                self.cfg.task_id,
                page_id=page_id,
                overlay=overlay,  # type: ignore[arg-type]
                action=action,
                evidence=evidence,
                result=result,  # type: ignore[arg-type]
                artifact_paths=arts,
                confidence=conf,
            )
        )

    def run(self) -> FlowResult:
        ensure_dpi_awareness()
        self.cfg.artifact_dir.mkdir(parents=True, exist_ok=True)
        until = self.cfg.until_step
        until_idx = len(STEP_ORDER) - 1 if until == Step.DONE else STEP_ORDER.index(until)

        self._bind_window_best_effort()
        start_step = self.cfg.from_step
        if self.cfg.auto_resume and self._win is not None and not self.cfg.dry_run:
            start_step = self._resume_step_from_ui(fallback=self.cfg.from_step)
            print(f"auto-resume → {start_step.value}")
        start_idx = STEP_ORDER.index(start_step)
        if start_step == Step.DONE:
            return FlowResult(True, Step.DONE, "already success / nothing to do", self.artifacts)

        for step in STEP_ORDER[start_idx:]:
            if step == Step.DONE:
                return FlowResult(True, Step.DONE, "booking UI path finished", self.artifacts)
            if step in (
                Step.SELECT_VENUE,
                Step.SELECT_DATE,
                Step.SELECT_SLOT,
                Step.CONFIRM_SLOT,
                Step.FILL_CAPTCHA,
            ) and not self.cfg.venue.strip():
                return FlowResult(
                    True,
                    Step.SELECT_SPORT,
                    "sport selected; pass --venue to continue past sport",
                    self.artifacts,
                )
            try:
                # Skip actions already satisfied by current page
                if self._should_skip_step(step):
                    print(f"skip {step.value} (page already past this step)")
                    self._snap(f"skip_{step.value}")
                else:
                    self._run_step(step)
                    self._log_ui(f"after_{step.value}")
            except Exception as exc:  # noqa: BLE001
                self._snap(f"fail_{step.value}")
                self._log_ui(f"fail_{step.value}")
                self._emit(action=f"fail_{step.value}", result="failed")
                return FlowResult(False, step, str(exc), self.artifacts)
            if STEP_ORDER.index(step) >= until_idx:
                return FlowResult(True, step, f"stopped at --until {step.value}", self.artifacts)
            time.sleep(self.cfg.step_delay)
        return FlowResult(True, Step.DONE, "ok", self.artifacts)

    def _bind_window_best_effort(self) -> None:
        """Prefer open mini-program; else WeChat main."""
        mp = window.find_miniprogram(title_hint=self.cfg.app_name)
        if mp is not None:
            self._win = mp
            if not self.cfg.dry_run:
                window.activate(mp)
            return
        if self.cfg.from_step == Step.ACTIVATE or self.cfg.auto_resume:
            try:
                self._win = window.find_wechat()
                if not self.cfg.dry_run:
                    window.activate(self._win)
            except RuntimeError:
                self._win = None

    def _snapshot(self) -> ui_state.UiSnapshot | None:
        if self._win is None:
            return None
        try:
            img = screen.grab_window(window.refresh(self._win))
        except Exception:  # noqa: BLE001
            return None
        try:
            return ui_state.classify(img)
        except Exception:  # noqa: BLE001
            return ui_state.UiSnapshot(Page.UNKNOWN, "")

    def _log_ui(self, tag: str) -> None:
        snap = self._snapshot()
        if snap is None:
            print(f"[{tag}] ui=(no window)")
            return
        print(f"[{tag}] {ui_state.snapshot_summary(snap)}")
        self._emit(action=tag, snap=snap)

    def _resume_step_from_ui(self, *, fallback: Step) -> Step:
        snap = self._snapshot()
        if snap is None:
            return fallback
        print(f"[resume] {ui_state.snapshot_summary(snap)}")
        self._snap(f"00_resume_{snap.page.value}")
        if snap.page == Page.SUCCESS:
            return Step.DONE
        name = ui_state.resume_step_name(snap)
        try:
            detected = Step(name)
        except ValueError:
            return fallback
        # Never go backwards past an explicit --from-step that is further ahead
        if STEP_ORDER.index(detected) < STEP_ORDER.index(fallback) and fallback != Step.ACTIVATE:
            return fallback
        # If user asked activate but we're already in-app, jump forward
        if fallback == Step.ACTIVATE or STEP_ORDER.index(detected) >= STEP_ORDER.index(fallback):
            return detected
        return fallback

    def _should_skip_step(self, step: Step) -> bool:
        """True when current page is already beyond what this step would accomplish."""
        if self.cfg.dry_run or not self.cfg.auto_resume:
            return False
        snap = self._snapshot()
        if snap is None:
            return False
        if snap.page == Page.SUCCESS:
            return step != Step.DONE
        # Already on confirm-order → skip everything before fill_captcha
        if snap.page == Page.CONFIRM_ORDER:
            return STEP_ORDER.index(step) < STEP_ORDER.index(Step.FILL_CAPTCHA)
        if snap.page == Page.SELECT_SLOT:
            if step in (Step.SELECT_DATE, Step.SELECT_VENUE, Step.SELECT_SPORT, Step.VENUE_BOOKING):
                return True
            if step == Step.SELECT_SLOT and snap.slot_cta == SlotCta.READY:
                return True  # jump to confirm_slot
        if snap.page == Page.VENUE_DETAIL and step in (
            Step.SELECT_VENUE,
            Step.SELECT_SPORT,
            Step.VENUE_BOOKING,
            Step.DISMISS_NOTICE,
        ):
            return True
        if snap.page == Page.VENUE_LIST and step in (
            Step.SELECT_SPORT,
            Step.VENUE_BOOKING,
            Step.DISMISS_NOTICE,
        ):
            return True
        if snap.page == Page.HOME and step in (Step.OPEN_APP, Step.SEARCH, Step.MINIPROGRAM, Step.DISCOVER):
            return True
        return False

    def _run_step(self, step: Step) -> None:
        {
            Step.ACTIVATE: self._step_activate,
            Step.DISCOVER: self._step_discover,
            Step.MINIPROGRAM: self._step_miniprogram,
            Step.SEARCH: self._step_search,
            Step.OPEN_APP: self._step_open_app,
            Step.VENUE_BOOKING: self._step_venue_booking,
            Step.DISMISS_NOTICE: self._step_dismiss_notice,
            Step.SELECT_SPORT: self._step_select_sport,
            Step.SELECT_VENUE: self._step_select_venue,
            Step.SELECT_DATE: self._step_select_date,
            Step.SELECT_SLOT: self._step_select_slot,
            Step.CONFIRM_SLOT: self._step_confirm_slot,
            Step.FILL_CAPTCHA: self._step_fill_captcha,
        }[step]()

    def _active(self) -> window.WindowInfo:
        if self._win is None:
            raise RuntimeError("window not activated")
        self._win = window.refresh(self._win)
        return self._win

    def _snap(self, name: str):
        if self._win is None:
            return None
        img = screen.grab_window(window.refresh(self._win))
        path = screen.save_bgr(self.cfg.artifact_dir / f"{name}.png", img)
        self.artifacts.append(path)
        return img

    def _click_xy(self, x: int, y: int, *, note: str = "") -> None:
        win = self._active()
        if self.cfg.dry_run:
            print(f"[dry-run] click ({x},{y}) {note}")
            return
        window.activate(win)
        input_ops.click_in_window(win, x, y)

    def _click_text(self, text: str, *, required: bool = True) -> bool:
        win = self._active()
        img = screen.grab_window(win)
        m = None
        if self.cfg.ocr:
            try:
                m = locate.find_text(img, text)
            except RuntimeError:
                if required and not self.cfg.dry_run and not self._template_fallback(text):
                    raise
        if m is None:
            m = self._template_fallback(text)
        if m is None:
            if self.cfg.dry_run:
                print(f"[dry-run] skip locate {text!r} (no OCR/template)")
                return True
            if required:
                raise RuntimeError(f"未在屏幕上找到文案/模板: {text!r}")
            return False
        if self.cfg.dry_run:
            print(f"[dry-run] click text {text!r} at ({m.x},{m.y}) score={m.score:.3f}")
            return True
        window.activate(win)
        input_ops.click_in_window(win, m.x, m.y)
        return True

    def _template_fallback(self, key: str):
        if not self.cfg.template_dir:
            return None
        names = {
            "发现": "discover.png",
            "小程序": "miniprogram.png",
            "场馆预约": "venue_booking.png",
            "厦大体育": "xmu_sports.png",
            "我知道了": "i_know.png",
            "确认预约": "confirm_booking.png",
            "确定预约": "submit_booking.png",
            "完成": "keyboard_done.png",
        }
        fname = names.get(key) or ("".join(c if c.isalnum() else "_" for c in key) + ".png")
        path = self.cfg.template_dir / fname
        if not path.is_file():
            return None
        win = self._active()
        img = screen.grab_window(win)
        return locate.match_template_file(img, path)

    def _step_activate(self) -> None:
        self._win = window.find_wechat()
        if self.cfg.dry_run:
            print(f"[dry-run] activate hwnd={self._win.hwnd} title={self._win.title!r}")
            return
        window.activate(self._win)
        self._snap("01_activate")

    def _step_discover(self) -> None:
        """Sidebar compass icon — no text label. Template + green-circle in left ROI."""
        win = self._active()
        img = screen.grab_window(win)
        tpl = self.cfg.template_dir
        m = wechat_nav.find_discover_icon(img, template_dir=tpl)
        if m is None and not self.cfg.dry_run:
            # One more try after short wait (window settle)
            time.sleep(0.4)
            img = screen.grab_window(window.refresh(win))
            m = wechat_nav.find_discover_icon(img, template_dir=tpl)
        if m is None:
            # OCR「发现」only works in the middle Discover *list* header, not sidebar —
            # still try as last resort after icon methods.
            if not self._click_text("发现", required=False):
                raise RuntimeError(
                    "未定位到侧栏「发现」指南针图标。请确认微信主窗口可见；"
                    "可检查 src-tauri/sidecars/vision/assets/templates/discover_icon.png"
                )
            self._snap("02_discover")
            return
        if self.cfg.dry_run:
            print(f"[dry-run] discover icon at ({m.x},{m.y}) score={m.score:.3f} via {m.label}")
        else:
            self._click_xy(m.x, m.y, note=m.label)
        time.sleep(0.8)
        self._snap("02_discover")

    def _step_miniprogram(self) -> None:
        """Middle Discover list: click OCR「小程序」row."""
        # Wait for Discover list header
        win = self._active()
        deadline = time.time() + 5
        while time.time() < deadline:
            img = screen.grab_window(window.refresh(win))
            if locate.find_text(img, "小程序") or locate.find_text(img, "朋友圈") or locate.find_text(img, "发现"):
                break
            time.sleep(0.35)
        self._click_text("小程序")
        time.sleep(0.8)
        self._snap("03_miniprogram")

    def _step_search(self) -> None:
        """Click magnifier on the mini-program panel (NOT chat search bar)."""
        win = self._active()
        img = screen.grab_window(win)
        m = wechat_nav.find_mp_search_icon(img, template_dir=self.cfg.template_dir)
        if m is None:
            raise RuntimeError("未找到小程序面板右上角搜索图标")
        if self.cfg.dry_run:
            print(f"[dry-run] mp search at ({m.x},{m.y}) via {m.label}; paste {self.cfg.app_name!r}")
            self._snap("04_search")
            return
        self._click_xy(m.x, m.y, note=m.label)
        time.sleep(0.45)
        window.activate(win)
        input_ops.paste_text(self.cfg.app_name)
        time.sleep(0.7)
        self._snap("04_search")

    def _step_open_app(self) -> None:
        """Click the first search result「厦大体育」(prefer 使用过的小程序)."""
        win = self._active()
        img = screen.grab_window(win)
        m = wechat_nav.find_first_app_result(img, self.cfg.app_name)
        if m is None:
            # Fallback plain OCR
            if not self._click_text(self.cfg.app_name, required=False):
                raise RuntimeError(f"搜索结果中未找到 {self.cfg.app_name!r}")
        else:
            if self.cfg.dry_run:
                print(f"[dry-run] open app at ({m.x},{m.y}) {m.label}")
            else:
                self._click_xy(m.x, m.y, note=m.label or self.cfg.app_name)
        time.sleep(self.cfg.step_delay)
        mp = window.find_miniprogram(title_hint=self.cfg.app_name)
        if mp is not None:
            self._win = mp
            if not self.cfg.dry_run:
                window.activate(self._win)
        self._snap("05_open_app")

    def _step_venue_booking(self) -> None:
        self._click_text("场馆预约")
        time.sleep(self.cfg.step_delay)
        self._snap("06_venue_booking")

    def _step_dismiss_notice(self) -> None:
        """【预约须知】 is optional — only click when the modal is present."""
        win = self._active()
        img = screen.grab_window(win)
        if self.cfg.ocr and not anchors.notice_present(img):
            if self.cfg.dry_run:
                print("[dry-run] no notice modal — skip")
            self._snap("07_no_notice")
            return
        clicked = self._click_text("我知道了", required=False)
        if not clicked:
            for alt in ("知道了", "同意"):
                if self._click_text(alt, required=False):
                    clicked = True
                    break
        if self.cfg.dry_run and not clicked:
            print("[dry-run] dismiss notice (optional, not found)")
        time.sleep(0.5)
        self._snap("07_after_notice")

    def _step_select_sport(self) -> None:
        sport = self.cfg.sport.strip()
        if not sport:
            raise RuntimeError("未配置 --sport")
        self._click_text(sport)
        time.sleep(self.cfg.step_delay)
        self._snap("08_select_sport")

    def _step_select_venue(self) -> None:
        venue = self.cfg.venue.strip()
        if not venue:
            raise RuntimeError("未配置 --venue")
        self._click_text(venue)
        time.sleep(self.cfg.step_delay)
        self._snap("09_select_venue")

    def _step_select_date(self) -> None:
        """Venue detail: click target day's「预约」; or slot page: click tomorrow date tab."""
        win = self._active()
        img = screen.grab_window(win)
        needles = dates.date_ocr_needles(self.cfg.date_offset)
        if self.cfg.dry_run:
            print(f"[dry-run] select date offset={self.cfg.date_offset} needles={needles}")
            self._snap("10_select_date")
            return

        # Already on multi-court「选择场次」— tap the date tab (e.g. 周五 / 09月18日)
        if locate.find_text(img, "选择场次"):
            day_match = None
            for needle in needles:
                day_match = locate.find_text(img, needle)
                if day_match:
                    break
            if day_match is None:
                raise RuntimeError(f"场次页未找到目标日期页签，尝试过: {needles}")
            self._click_xy(day_match.x, day_match.y, note=f"date tab {needles}")
            time.sleep(self.cfg.step_delay)
            self._snap("10_select_date")
            return

        day_match = None
        for needle in needles:
            day_match = locate.find_text(img, needle)
            if day_match:
                break
        if day_match is None:
            raise RuntimeError(f"未找到目标日期文案，尝试过: {needles}")

        # Venue cards use「预定」; some sports use「预约」/「立即预约」
        book_hits = []
        for h in locate.ocr_image(img):
            t = h.text.strip()
            if any(bad in t for bad in ("须知", "确认", "确定", "暂无", "暂满", "类型")):
                continue
            if t in ("预定", "预约", "立即预约", "立即预定") or (
                (t.endswith("预约") or t.endswith("预定")) and "暂" not in t
            ):
                book_hits.append(h)
        if not book_hits:
            # Only fail as full if the *target* day card itself is 暂无/暂满
            near_full = [
                h
                for h in locate.ocr_image(img)
                if ("暂无" in h.text or "暂满" in h.text)
                and abs(h.x - day_match.x) < 120
                and abs(h.y - day_match.y) < 100
            ]
            if near_full:
                raise RuntimeError(f"目标日 offset={self.cfg.date_offset} 暂不可约")
            raise RuntimeError("未找到日期卡片上的「预定/预约」")

        def dist2(h: locate.OcrHit) -> float:
            dx = h.x - day_match.x
            dy = h.y - day_match.y
            if dy < -10:
                return 1e12
            return dx * dx + dy * dy

        best = min(book_hits, key=dist2)
        if dist2(best) >= 1e12:
            raise RuntimeError("「预定/预约」不在目标日期卡片附近")
        self._click_xy(best.x, best.y, note="date 预定")
        time.sleep(self.cfg.step_delay)
        self._snap("10_select_date")

    def _step_select_slot(self) -> None:
        """On「选择场次」: lock target time row, L→R courts; optional nearest-time fallback + scroll."""
        win = self._active()
        img = screen.grab_window(win)
        gym_mode = "健身" in (self.cfg.sport or "")
        if self.cfg.dry_run:
            cands = (
                slots.find_available_slot_candidates(
                    img,
                    prefer_time=self.cfg.prefer_time,
                    prefer_court=self.cfg.prefer_court,
                    gym_mode=gym_mode,
                    allow_time_fallback=self.cfg.allow_time_fallback,
                    time_fallback_order=self.cfg.time_fallback_order,
                    court_offset=self.cfg.court_offset,
                )
                if self.cfg.ocr
                else []
            )
            print(
                f"[dry-run] select_slot prefer_time={self.cfg.prefer_time!r} "
                f"prefer_court={self.cfg.prefer_court!r} court_offset={self.cfg.court_offset} "
                f"candidates={len(cands)} gym={gym_mode} "
                f"fallback={self.cfg.allow_time_fallback}/{self.cfg.time_fallback_order}"
            )
            self._snap("11_select_slot")
            return

        img = screen.grab_window(win)
        snap = ui_state.classify(img)
        print(f"[select_slot] {ui_state.snapshot_summary(snap)}")
        if snap.page == Page.CONFIRM_ORDER:
            print("already on confirm_order — select_slot no-op")
            self._snap("11_select_slot_already_confirm")
            return
        if snap.page != Page.SELECT_SLOT:
            time.sleep(2.0)
            img = screen.grab_window(window.refresh(win))
            snap = ui_state.classify(img)
            print(f"[select_slot retry] {ui_state.snapshot_summary(snap)}")
        if snap.page == Page.CONFIRM_ORDER:
            self._snap("11_select_slot_already_confirm")
            return
        if snap.page != Page.SELECT_SLOT:
            self._snap("11_select_slot_timeout")
            raise RuntimeError(f"未进入「选择场次」页 (got {snap.page.value})")
        if snap.slot_cta == SlotCta.READY:
            print("slot already selected — select_slot no-op")
            self._snap("11_select_slot_already_ready")
            return

        if self.cfg.date_offset >= 1:
            needles = dates.date_ocr_needles(self.cfg.date_offset)
            for needle in needles:
                tab = locate.find_text(img, needle)
                if tab and tab.y < img.shape[0] * 0.22:
                    self._click_xy(tab.x, tab.y, note=f"ensure date tab {needle}")
                    time.sleep(0.9)
                    img = screen.grab_window(window.refresh(win))
                    break

        cands = self._collect_slot_candidates(img, gym_mode=gym_mode)
        if not cands and self.cfg.prefer_time.strip() and not self.cfg.allow_time_fallback:
            self._snap("11_no_slots_at_target_time")
            raise RuntimeError(
                f"目标时段 {self.cfg.prefer_time!r} 无可用场地（未开启换时段）"
            )
        if not cands:
            time.sleep(0.8)
            img = screen.grab_window(window.refresh(win))
            cands = self._collect_slot_candidates(img, gym_mode=gym_mode)
        if not cands and not gym_mode:
            # Horizontal scroll to reveal more court columns, then retry
            for scroll_i in range(4):
                self._scroll_slot_grid_right(win)
                time.sleep(0.7)
                img = screen.grab_window(window.refresh(win))
                cands = self._collect_slot_candidates(img, gym_mode=gym_mode)
                if cands:
                    print(f"[select_slot] found after scroll #{scroll_i + 1}")
                    break
        if not cands:
            self._snap("11_no_mint_cells")
            raise RuntimeError("场次页没有检测到「可预约」色块（可能全满或日期不对）")

        last_err = ""
        tried: set[tuple[int, int]] = set()
        attempt = 0
        scroll_budget = 0 if gym_mode else 4
        while attempt < 16:
            if not cands and scroll_budget > 0:
                self._scroll_slot_grid_right(win)
                scroll_budget -= 1
                time.sleep(0.7)
                img = screen.grab_window(window.refresh(win))
                rebuilt = self._collect_slot_candidates(img, gym_mode=gym_mode)
                cands = [
                    c
                    for c in rebuilt
                    if all(abs(c.click_x - tx) > 20 or abs(c.click_y - ty) > 18 for tx, ty in tried)
                ]
                continue
            if not cands:
                break
            cand = cands[0]
            attempt += 1
            tried.add((cand.click_x, cand.click_y))
            note = (
                f"slot t={cand.start_time} col={cand.court_index} "
                f"label={cand.court_label or '-'} state={cand.state.value}"
            )
            self._click_xy(cand.click_x, cand.click_y, note=note)
            time.sleep(2.0)
            img2 = screen.grab_window(window.refresh(win))
            self._snap(f"11_select_slot_try{attempt}")
            snap2 = ui_state.classify(img2)
            print(f"[select_slot after click] {ui_state.snapshot_summary(snap2)}")
            if ui_state.can_confirm_slot(snap2) or anchors.slot_looks_selected(img2):
                self._snap("11_select_slot")
                return
            last_err = "点击后仍是「请先选择场次」"
            if snap2.page != Page.SELECT_SLOT:
                break
            rebuilt = self._collect_slot_candidates(img2, gym_mode=gym_mode, prefer_court="")
            cands = [
                c
                for c in rebuilt
                if all(abs(c.click_x - tx) > 20 or abs(c.click_y - ty) > 18 for tx, ty in tried)
            ]
        raise RuntimeError(last_err or f"目标时段场地均不可用（尝试 {attempt} 次）")

    def _collect_slot_candidates(
        self,
        img,
        *,
        gym_mode: bool,
        prefer_court: str | None = None,
    ):
        court = self.cfg.prefer_court if prefer_court is None else prefer_court
        return slots.find_available_slot_candidates(
            img,
            prefer_time=self.cfg.prefer_time,
            prefer_court=court,
            gym_mode=gym_mode,
            allow_time_fallback=self.cfg.allow_time_fallback,
            time_fallback_order=self.cfg.time_fallback_order,
            court_offset=self.cfg.court_offset,
        )

    def _scroll_slot_grid_right(self, win: window.WindowInfo) -> None:
        """Drag left on the grid so columns to the right become visible."""
        w = win.width
        h = win.height
        # Mid-grid horizontal drag: right → left
        x0 = int(w * 0.82)
        x1 = int(w * 0.28)
        y = int(h * 0.42)
        if self.cfg.dry_run:
            print(f"[dry-run] scroll slot grid ({x0},{y})→({x1},{y})")
            return
        window.activate(win)
        input_ops.drag_in_window(win, x0, y, x1, y, steps=14, settle=0.35)

    def _step_confirm_slot(self) -> None:
        """Bottom「确认预约」— only when SELECT_SLOT + READY."""
        win = self._active()
        img = screen.grab_window(win)
        if self.cfg.dry_run:
            print("[dry-run] confirm_slot → 确认预约")
            self._snap("12_confirm_slot")
            return
        snap = ui_state.classify(img)
        print(f"[confirm_slot] {ui_state.snapshot_summary(snap)}")
        if snap.page == Page.CONFIRM_ORDER:
            print("already on confirm_order — confirm_slot no-op")
            self._snap("12_confirm_slot_already")
            return
        if snap.page != Page.SELECT_SLOT:
            raise RuntimeError(f"confirm_slot 期望选择场次页, got {snap.page.value}")
        if snap.slot_cta == SlotCta.NEED_SELECT:
            raise RuntimeError("场次未选中，底部仍是「请先选择场次」")
        btn = anchors.find_confirm_button(img)
        if btn is None:
            raise RuntimeError("未找到底部「确认预约」按钮")
        self._click_xy(btn.x, btn.y, note="确认预约")
        time.sleep(2.0)
        img2 = screen.grab_window(window.refresh(win))
        snap2 = ui_state.classify(img2)
        print(f"[after confirm_slot] {ui_state.snapshot_summary(snap2)}")
        self._snap("12_confirm_slot")
        if snap2.page != Page.CONFIRM_ORDER and not locate.find_text(img2, "验证码"):
            raise RuntimeError(f"点确认预约后未进入验证码页 (got {snap2.page.value})")

    def _step_fill_captcha(self) -> None:
        """确认预约页：状态机驱动 输入→完成→确定预约."""
        win = self._active()
        if self.cfg.dry_run:
            img = screen.grab_window(win)
            row = None
            if self.cfg.ocr:
                try:
                    row = anchors.find_captcha_row(img)
                except RuntimeError:
                    row = None
            print(f"[dry-run] fill_captcha row_found={row is not None}")
            if row is not None:
                captcha_kb.fill_captcha_and_submit(win, dry_run=True, max_tries=1)
            else:
                print("[dry-run] skip captcha ritual (no OCR anchor on current screen)")
            self._snap("13_fill_captcha")
            return

        img = screen.grab_window(win)
        snap = ui_state.classify(img)
        print(f"[fill_captcha] {ui_state.snapshot_summary(snap)}")
        if snap.page == Page.SUCCESS:
            self._snap("13_already_success")
            return
        if snap.page != Page.CONFIRM_ORDER:
            time.sleep(2.0)
            img = screen.grab_window(window.refresh(win))
            snap = ui_state.classify(img)
            print(f"[fill_captcha retry] {ui_state.snapshot_summary(snap)}")
        if snap.page == Page.SUCCESS:
            self._snap("13_already_success")
            return
        if snap.page != Page.CONFIRM_ORDER:
            self._snap("13_fill_captcha_timeout")
            raise RuntimeError(f"未进入确认预约/验证码页 (got {snap.page.value})")

        code = captcha_kb.fill_captcha_and_submit(
            win, dry_run=False, max_tries=self.cfg.captcha_retries
        )
        print(f"captcha submitted code={code!r}")
        time.sleep(2.0)
        snap = ui_state.classify(screen.grab_window(window.refresh(win)))
        print(f"[after fill_captcha] {ui_state.snapshot_summary(snap)}")
        self._snap("13_after_submit")
        self._emit(action="after_fill_captcha", result="success", snap=snap)
        # Stack may be …→选场次→确认→我的预约; pop until sport entrance when asked
        if self.cfg.pop_to_sport_list:
            from xmu_vision.nav import pop_until

            pr = pop_until(win, Page.VENUE_SPORT_LIST, max_pops=8)
            print(f"[pop_until] ok={pr.ok} pops={pr.pops} page={pr.page.value} {pr.message}")
            self._emit(
                action="pop_until_sport",
                result="success" if pr.ok else "need_human",
                snap=pr.snapshots[-1] if pr.snapshots else None,
            )
            self._snap("14_after_pop")
