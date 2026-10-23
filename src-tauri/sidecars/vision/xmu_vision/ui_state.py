"""UI page + sub-state classifier for vision booking (body-fingerprint guards).

Page identity comes from body landmarks and mutually exclusive control combos.
PC mini-program chrome titles are debug-only — they lag behind the route stack.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum

import cv2
import numpy as np

from xmu_vision import anchors, locate
from xmu_vision.locate import ocr_image


class Page(str, Enum):
    UNKNOWN = "unknown"
    WECHAT_MAIN = "wechat_main"
    WECHAT_DISCOVER = "wechat_discover"
    WECHAT_MP_PANEL = "wechat_mp_panel"
    WECHAT_MP_SEARCH = "wechat_mp_search"
    HOME = "home"
    VENUE_SPORT_LIST = "venue_sport_list"
    VENUE_LIST = "venue_list"
    VENUE_DETAIL = "venue_detail"
    SELECT_SLOT = "select_slot"
    CONFIRM_ORDER = "confirm_order"
    MY_RESERVATIONS = "my_reservations"
    ORDER_DETAIL = "order_detail"
    CANCEL_MODAL = "cancel_modal"
    SUCCESS = "success"


class Overlay(str, Enum):
    NONE = "none"
    KEYBOARD = "keyboard"
    TOAST = "toast"
    MODAL = "modal"
    NOTICE = "notice"


class SlotCta(str, Enum):
    NEED_SELECT = "need_select"
    READY = "ready"
    UNKNOWN = "unknown"


class CaptchaPhase(str, Enum):
    IDLE_EMPTY = "idle_empty"
    KEYBOARD_OPEN_EMPTY = "kb_open_empty"
    KEYBOARD_OPEN_PARTIAL = "kb_open_partial"
    KEYBOARD_OPEN_FULL = "kb_open_full"
    IDLE_FULL = "idle_full"
    TOAST_NEED_INPUT = "toast_need_input"
    TOAST_ERROR = "toast_error"
    UNKNOWN = "unknown"


@dataclass(frozen=True)
class UiSnapshot:
    page: Page
    title_hint: str
    slot_cta: SlotCta = SlotCta.UNKNOWN
    captcha: CaptchaPhase = CaptchaPhase.UNKNOWN
    keyboard_open: bool = False
    input_chars: str = ""
    has_mint_slots: bool = False
    toast_error: bool = False
    toast_need_input: bool = False
    overlay: Overlay = Overlay.NONE
    evidence: tuple[str, ...] = ()
    confidence: float = 0.0


def _title_hits(image_bgr: np.ndarray) -> list[str]:
    """OCR of top chrome — debug only; never used as sole page identity."""
    h = image_bgr.shape[0]
    texts: list[str] = []
    for hit in ocr_image(image_bgr):
        if hit.y > h * 0.14:
            continue
        if hit.score < 0.5:
            continue
        t = hit.text.strip()
        if t:
            texts.append(t)
    return texts


def _body_has(image_bgr: np.ndarray, needle: str, *, min_y_frac: float = 0.0, max_y_frac: float = 1.0) -> bool:
    h = image_bgr.shape[0]
    y0, y1 = int(h * min_y_frac), int(h * max_y_frac)
    for hit in ocr_image(image_bgr):
        if hit.score < 0.5:
            continue
        if needle not in hit.text:
            continue
        if y0 <= hit.y <= y1:
            return True
    return False


def has_sport_icon_grid(image_bgr: np.ndarray) -> bool:
    """Body fingerprint for 运动入口: several sport labels in mid/upper content."""
    sports = ("乒乓球", "羽毛球", "篮球", "足球", "排球", "网球", "瑜伽", "健身房")
    h = image_bgr.shape[0]
    found: list[str] = []
    for hit in ocr_image(image_bgr):
        if hit.score < 0.45:
            continue
        # Sport grid sits below blue header, above「所有场馆」list — roughly 18–55%
        if hit.y < h * 0.14 or hit.y > h * 0.58:
            continue
        for s in sports:
            if s in hit.text and s not in found:
                found.append(s)
                break
    return len(found) >= 2


def has_venue_list_rows(image_bgr: np.ndarray) -> bool:
    """True when content looks like a venue list (not merely the word 场馆列表)."""
    # Confirm-order form shows the venue name; never treat that as list rows.
    if locate.find_text(image_bgr, "验证码") or locate.find_text(image_bgr, "场馆信息"):
        return False
    if locate.find_text(image_bgr, "所有场馆"):
        return True
    # Venue rows often contain 校区 + chevron context; require list-ish markers
    campus = locate.find_text(image_bgr, "校区")
    if campus and not has_sport_icon_grid(image_bgr):
        # Pure venue list pages still have multiple venue name hits
        names = 0
        for hit in ocr_image(image_bgr):
            if hit.score < 0.5:
                continue
            if "场馆" in hit.text or "球馆" in hit.text or "室" in hit.text:
                names += 1
        return names >= 2
    return False


def has_my_reservations_list(image_bgr: np.ndarray) -> bool:
    """List body for 我的预约 — never the header link alone."""
    status = locate.find_text(image_bgr, "已预约") or locate.find_text(image_bgr, "已取消")
    footer = (
        locate.find_text(image_bgr, "已经到底")
        or locate.find_text(image_bgr, "已核销")
        or locate.find_text(image_bgr, "暂无预约")
        or locate.find_text(image_bgr, "没有预约")
    )
    return bool(status and footer)


def keyboard_is_open(image_bgr: np.ndarray) -> bool:
    """Detect soft keyboard via lower ROI only — never full-screen「取消」(modal)."""
    h, w = image_bgr.shape[:2]
    y0 = int(h * 0.52)
    roi = image_bgr[y0:h, :]
    if roi.size == 0:
        return False
    # Soft-key digit row + toolbar 完成/取消, or red backspace blob
    if anchors.keyboard_backspace(image_bgr) is not None:
        return True
    done = locate.find_text(roi, "完成")
    cancel = locate.find_text(roi, "取消")
    digit = locate.find_text(roi, "1") or locate.find_text(roi, "2")
    if done and digit:
        return True
    if cancel and digit:
        # Modal cancel is usually mid-screen; keyboard cancel is in lower half with digits
        return True
    # Green「完成」color strip without OCR
    if anchors.keyboard_done(image_bgr) is not None and digit:
        return True
    return False


def read_captcha_input_text(image_bgr: np.ndarray) -> str:
    """OCR characters already typed into the four slots right of the captcha image."""
    row = anchors.find_captcha_row(image_bgr)
    if row is None:
        return ""
    _label, cap, (ix, iy) = row
    h, w = image_bgr.shape[:2]
    x0 = min(w - 1, cap.x1 + 2)
    x1 = min(w, max(ix + 40, cap.x1 + int(cap.w * 1.2)))
    y0 = max(0, cap.y0 - 4)
    y1 = min(h, cap.y1 + 8)
    strip = image_bgr[y0:y1, x0:x1]
    if strip.size == 0:
        return ""
    big = cv2.resize(strip, None, fx=2.5, fy=2.5, interpolation=cv2.INTER_CUBIC)
    try:
        hits = ocr_image(big)
    except RuntimeError:
        return ""
    raw = "".join(h.text for h in hits)
    return "".join(c for c in raw if c.isalnum())[:4]


def classify_captcha_phase(image_bgr: np.ndarray) -> CaptchaPhase:
    kind = anchors.captcha_toast_kind(image_bgr)
    if kind == "need_input":
        return CaptchaPhase.TOAST_NEED_INPUT
    if kind == "wrong":
        return CaptchaPhase.TOAST_ERROR
    kb = keyboard_is_open(image_bgr)
    chars = read_captcha_input_text(image_bgr)
    n = len(chars)
    if kb:
        if n >= 4:
            return CaptchaPhase.KEYBOARD_OPEN_FULL
        if n > 0:
            return CaptchaPhase.KEYBOARD_OPEN_PARTIAL
        return CaptchaPhase.KEYBOARD_OPEN_EMPTY
    if n >= 4:
        return CaptchaPhase.IDLE_FULL
    return CaptchaPhase.IDLE_EMPTY


def classify_slot_cta(image_bgr: np.ndarray) -> SlotCta:
    if locate.find_text(image_bgr, "请先选择场次"):
        return SlotCta.NEED_SELECT
    btn = anchors.find_confirm_button(image_bgr)
    if btn and btn.label in ("确认预约", "cta_blue"):
        return SlotCta.READY
    if locate.find_text(image_bgr, "确认预约"):
        return SlotCta.UNKNOWN
    return SlotCta.UNKNOWN


def _snap(
    page: Page,
    title_hint: str,
    *,
    evidence: list[str] | tuple[str, ...] = (),
    confidence: float = 0.8,
    overlay: Overlay = Overlay.NONE,
    **kwargs,
) -> UiSnapshot:
    return UiSnapshot(
        page,
        title_hint,
        evidence=tuple(evidence),
        confidence=confidence,
        overlay=overlay,
        **kwargs,
    )


def classify(image_bgr: np.ndarray) -> UiSnapshot:
    """Classify page from body landmarks. title_hint is debug-only."""
    titles = _title_hits(image_bgr)
    title_blob = " ".join(titles)

    # --- Overlays / modals first ---
    if find_hit_any(image_bgr, ("你确定要取消预约吗", "确定要取消预约吗", "确定要取消预约")):
        return _snap(
            Page.CANCEL_MODAL,
            title_blob or "取消确认",
            evidence=["modal:你确定要取消预约吗"],
            confidence=0.95,
            overlay=Overlay.MODAL,
        )
    if (
        locate.find_text(image_bgr, "提示")
        and locate.find_text(image_bgr, "确定")
        and locate.find_text(image_bgr, "取消")
        and locate.find_text(image_bgr, "取消预约")
    ):
        return _snap(
            Page.CANCEL_MODAL,
            title_blob or "取消确认",
            evidence=["modal:提示+确定+取消+取消预约"],
            confidence=0.9,
            overlay=Overlay.MODAL,
        )

    if locate.find_text(image_bgr, "预约成功"):
        return _snap(Page.SUCCESS, title_blob or "预约成功", evidence=["text:预约成功"], confidence=0.95)

    # My reservations — list structure required (NOT the header link「我的预约」)
    if has_my_reservations_list(image_bgr):
        return _snap(
            Page.MY_RESERVATIONS,
            title_blob or "我的预约",
            evidence=["list:已预约|已取消", "footer:到底|核销|暂无"],
            confidence=0.9,
        )

    # Order detail
    if (
        locate.find_text(image_bgr, "预约详情")
        or locate.find_text(image_bgr, "订单详情")
        or (
            locate.find_text(image_bgr, "取消预约")
            and (locate.find_text(image_bgr, "进场凭证") or locate.find_text(image_bgr, "预约编号"))
        )
    ):
        return _snap(
            Page.ORDER_DETAIL,
            title_blob or "订单详情",
            evidence=["order:详情|凭证|编号"],
            confidence=0.88,
        )

    # Confirm order — body「验证码」+ second feature (title alone is NOT enough).
    # Soft keyboard covers bottom CTA「确定预约」; form labels /「确认预约」still count.
    if anchors.find_hit(image_bgr, "验证码") and (
        locate.find_text(image_bgr, "确定预约")
        or locate.find_text(image_bgr, "订单信息")
        or locate.find_text(image_bgr, "确认预约")
        or locate.find_text(image_bgr, "场馆信息")
        or locate.find_text(image_bgr, "预约类型")
        or locate.find_text(image_bgr, "订单价格")
        or locate.find_text(image_bgr, "预约项目")
    ):
        kb = keyboard_is_open(image_bgr)
        chars = read_captcha_input_text(image_bgr)
        phase = classify_captcha_phase(image_bgr)
        overlay = Overlay.KEYBOARD if kb else (Overlay.TOAST if phase in (CaptchaPhase.TOAST_ERROR, CaptchaPhase.TOAST_NEED_INPUT) else Overlay.NONE)
        return _snap(
            Page.CONFIRM_ORDER,
            title_blob or "确认预约",
            evidence=["anchor:验证码", "cta:确定预约|订单字段|确认预约"],
            confidence=0.92,
            captcha=phase,
            keyboard_open=kb,
            input_chars=chars,
            toast_error=phase == CaptchaPhase.TOAST_ERROR,
            toast_need_input=phase == CaptchaPhase.TOAST_NEED_INPUT,
            overlay=overlay,
        )

    # Select slot
    if locate.find_text(image_bgr, "选择场次") or (
        locate.find_text(image_bgr, "可预约") and locate.find_text(image_bgr, "不可选")
    ):
        cells = anchors.find_available_slot_cells(image_bgr)
        return _snap(
            Page.SELECT_SLOT,
            title_blob or "选择场次",
            evidence=["legend:可预约/不可选|选择场次"],
            confidence=0.9,
            slot_cta=classify_slot_cta(image_bgr),
            has_mint_slots=bool(cells),
        )

    # Venue detail
    if locate.find_text(image_bgr, "场馆详情") or (
        (locate.find_text(image_bgr, "预定") or locate.find_text(image_bgr, "暂无"))
        and (locate.find_text(image_bgr, "场馆简介") or locate.find_text(image_bgr, "今天"))
    ):
        return _snap(
            Page.VENUE_DETAIL,
            title_blob or "场馆详情",
            evidence=["detail:预定|简介"],
            confidence=0.85,
        )

    # Sport entrance BEFORE venue_list — grid beats stale title /「场馆列表」word /「我的预约」link
    if has_sport_icon_grid(image_bgr):
        return _snap(
            Page.VENUE_SPORT_LIST,
            title_blob or "选运动",
            evidence=["grid:sport_icons≥2"],
            confidence=0.93,
        )
    # Fallback: classic two-sport OCR without y filter (older layouts)
    if locate.find_text(image_bgr, "乒乓球") and locate.find_text(image_bgr, "羽毛球"):
        return _snap(
            Page.VENUE_SPORT_LIST,
            title_blob or "选运动",
            evidence=["text:乒乓球+羽毛球"],
            confidence=0.8,
        )

    # Venue list — rows + NO sport grid; never title alone
    if has_venue_list_rows(image_bgr) and not has_sport_icon_grid(image_bgr):
        return _snap(
            Page.VENUE_LIST,
            title_blob or "场馆列表",
            evidence=["list:venue_rows", "no_sport_grid"],
            confidence=0.85,
        )
    # Weak fallback: body content title「场馆列表」below chrome, still no sport grid
    if _body_has(image_bgr, "场馆列表", min_y_frac=0.12, max_y_frac=0.4) and not has_sport_icon_grid(
        image_bgr
    ):
        return _snap(
            Page.VENUE_LIST,
            title_blob or "场馆列表",
            evidence=["body:场馆列表", "no_sport_grid"],
            confidence=0.7,
        )

    if locate.find_text(image_bgr, "场馆预约") and locate.find_text(image_bgr, "首页"):
        return _snap(Page.HOME, title_blob or "首页", evidence=["home:场馆预约+首页"], confidence=0.85)

    if locate.find_text(image_bgr, "小程序") and locate.find_text(image_bgr, "朋友圈"):
        return _snap(
            Page.WECHAT_DISCOVER,
            title_blob or "发现",
            evidence=["wechat:小程序+朋友圈"],
            confidence=0.8,
        )

    return _snap(Page.UNKNOWN, title_blob, evidence=["unmatched"], confidence=0.0)


def find_hit_any(image_bgr: np.ndarray, needles: tuple[str, ...]):
    for n in needles:
        h = anchors.find_hit(image_bgr, n)
        if h is not None:
            return h
    return None


def require_page(image_bgr: np.ndarray, *pages: Page) -> UiSnapshot:
    snap = classify(image_bgr)
    if snap.page not in pages:
        raise RuntimeError(
            f"页面状态不符: got={snap.page.value} want={[p.value for p in pages]} "
            f"title={snap.title_hint!r} evidence={list(snap.evidence)}"
        )
    return snap


def can_submit_order(snap: UiSnapshot) -> bool:
    return (
        snap.page == Page.CONFIRM_ORDER
        and snap.captcha == CaptchaPhase.IDLE_FULL
        and not snap.keyboard_open
        and len(snap.input_chars) >= 4
        and not snap.toast_error
        and not snap.toast_need_input
    )


def can_confirm_slot(snap: UiSnapshot) -> bool:
    return snap.page == Page.SELECT_SLOT and snap.slot_cta == SlotCta.READY


PAGE_RESUME_STEP: dict[Page, str] = {
    Page.SUCCESS: "done",
    Page.CONFIRM_ORDER: "fill_captcha",
    Page.SELECT_SLOT: "select_slot",
    Page.VENUE_DETAIL: "select_date",
    Page.VENUE_LIST: "select_venue",
    Page.VENUE_SPORT_LIST: "select_sport",
    Page.HOME: "venue_booking",
    Page.WECHAT_DISCOVER: "miniprogram",
    Page.WECHAT_MP_PANEL: "search",
    Page.WECHAT_MP_SEARCH: "open_app",
    Page.WECHAT_MAIN: "discover",
    Page.UNKNOWN: "activate",
}


def resume_step_name(snap: UiSnapshot) -> str:
    if snap.page == Page.SELECT_SLOT and snap.slot_cta == SlotCta.READY:
        return "confirm_slot"
    return PAGE_RESUME_STEP.get(snap.page, "activate")


def snapshot_summary(snap: UiSnapshot) -> str:
    parts = [
        f"page={snap.page.value}",
        f"title={snap.title_hint!r}",
        f"conf={snap.confidence:.2f}",
    ]
    if snap.evidence:
        parts.append("ev=" + ",".join(snap.evidence[:3]))
    if snap.overlay != Overlay.NONE:
        parts.append(f"overlay={snap.overlay.value}")
    if snap.page == Page.SELECT_SLOT:
        parts.append(f"slot_cta={snap.slot_cta.value}")
        parts.append(f"mint={snap.has_mint_slots}")
    if snap.page == Page.CONFIRM_ORDER:
        parts.append(f"captcha={snap.captcha.value}")
        parts.append(f"kb={snap.keyboard_open}")
        parts.append(f"input={snap.input_chars!r}")
    return " ".join(parts)
