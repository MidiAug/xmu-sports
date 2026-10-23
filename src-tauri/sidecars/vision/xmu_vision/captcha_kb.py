"""Confirm-page captcha: focus input → type → 完成 → 确定预约.

Anchors only (OCR / color) — never screen-percent clicks. See anchors.py.
"""

from __future__ import annotations

import time
from dataclasses import dataclass

import cv2
import numpy as np

from xmu_vision import anchors, input_ops, locate, screen, ui_state, window
from xmu_vision.ui_state import Page


class SubmitOutcome:
    SUCCESS = "success"
    CAPTCHA_EMPTY = "captcha_empty"
    CAPTCHA_WRONG = "captcha_wrong"
    UNKNOWN_AFTER_SUBMIT = "unknown_after_submit"
    LEFT_CONFIRM = "left_confirm"


@dataclass
class SubmitObserveResult:
    outcome: str
    toast_kind: str | None
    page: Page
    frames_checked: int
    evidence: list[str]


@dataclass
class KeyMap:
    mapping: dict[str, tuple[int, int]]

    def path_for(self, text: str) -> list[tuple[int, int]]:
        pts: list[tuple[int, int]] = []
        for ch in text:
            key = ch if ch in self.mapping else ch.lower()
            if key not in self.mapping:
                key = ch.upper()
            if key not in self.mapping:
                raise KeyError(f"keyboard missing key for {ch!r}; have {sorted(self.mapping)}")
            pts.append(self.mapping[key])
        return pts


def build_keymap_from_ocr(keyboard_bgr: np.ndarray, *, min_score: float = 0.4) -> KeyMap:
    mapping: dict[str, tuple[int, int]] = {}
    for hit in locate.ocr_image(keyboard_bgr):
        if hit.score < min_score:
            continue
        t = hit.text.strip()
        if len(t) != 1:
            continue
        mapping[t] = (hit.x, hit.y)
        mapping[t.lower()] = (hit.x, hit.y)
        mapping[t.upper()] = (hit.x, hit.y)
    _fill_qwerty_gaps(mapping, keyboard_bgr)
    return KeyMap(mapping)


def _fill_qwerty_gaps(mapping: dict[str, tuple[int, int]], image_bgr: np.ndarray) -> None:
    """Infer missing letter keys from neighbors (OCR often misreads u/i/o/j/f)."""
    rows = [
        list("1234567890"),
        list("qwertyuiop"),
        list("asdfghjkl"),
        list("zxcvbnm"),
    ]
    h, w = image_bgr.shape[:2]
    for row in rows:
        known = [(i, mapping[ch.lower()]) for i, ch in enumerate(row) if ch.lower() in mapping]
        if len(known) < 2:
            continue
        # Interpolate missing indices along the row
        xs = [p[0] for _, p in known]
        ys = [p[1] for _, p in known]
        idxs = [i for i, _ in known]
        y_med = int(sorted(ys)[len(ys) // 2])
        for i, ch in enumerate(row):
            key = ch.lower()
            if key in mapping:
                continue
            # linear interp by index between nearest known neighbors
            left = max((j for j in idxs if j < i), default=None)
            right = min((j for j in idxs if j > i), default=None)
            if left is None and right is None:
                continue
            if left is None:
                # extrapolate left of first known
                i1, i2 = idxs[0], idxs[1] if len(idxs) > 1 else idxs[0]
                x1, x2 = mapping[row[i1].lower()][0], mapping[row[i2].lower()][0]
                step = (x2 - x1) / max(1, i2 - i1)
                x = int(x1 + step * (i - i1))
            elif right is None:
                i1, i2 = idxs[-2] if len(idxs) > 1 else idxs[-1], idxs[-1]
                x1, x2 = mapping[row[i1].lower()][0], mapping[row[i2].lower()][0]
                step = (x2 - x1) / max(1, i2 - i1)
                x = int(x2 + step * (i - i2))
            else:
                x1 = mapping[row[left].lower()][0]
                x2 = mapping[row[right].lower()][0]
                t = (i - left) / (right - left)
                x = int(x1 + t * (x2 - x1))
            x = min(w - 4, max(4, x))
            y = y_med
            mapping[key] = (x, y)
            mapping[key.upper()] = (x, y)
            mapping[ch] = (x, y)


def crop_captcha_image(image_bgr: np.ndarray, captcha_box: anchors.Box) -> np.ndarray:
    h, w = image_bgr.shape[:2]
    x0, y0 = max(0, captcha_box.x0), max(0, captcha_box.y0)
    x1, y1 = min(w, captcha_box.x1), min(h, captcha_box.y1)
    return image_bgr[y0:y1, x0:x1].copy()


def ocr_captcha_crop(crop_bgr: np.ndarray) -> str:
    """Recognize 4-char captcha; prefer ddddocr (same as archive http-probe), RapidOCR fallback."""
    if crop_bgr.size == 0:
        return ""
    # Upscale small UI crops for OCR models
    h, w = crop_bgr.shape[:2]
    scale = 2.5 if max(h, w) < 160 else 1.5
    big = cv2.resize(crop_bgr, None, fx=scale, fy=scale, interpolation=cv2.INTER_CUBIC)

    code = _ddddocr_bytes(big)
    if len(code) >= 4:
        return code[:4].upper()

    # RapidOCR fallback with simple binarization variants
    for im in _captcha_variants(big):
        try:
            hits = locate.ocr_image(im)
        except RuntimeError:
            continue
        raw = "".join(h.text for h in hits)
        cand = "".join(c for c in raw if c.isalnum())
        if len(cand) >= 4:
            return cand[:4].upper()
        if len(cand) > len(code):
            code = cand
    return code.upper()


def _ddddocr_bytes(image_bgr: np.ndarray) -> str:
    try:
        import ddddocr
    except ImportError:
        return ""
    ok, buf = cv2.imencode(".png", image_bgr)
    if not ok:
        return ""
    raw = buf.tobytes()
    last = ""
    for use_beta in (True, False):
        try:
            ocr = ddddocr.DdddOcr(show_ad=False, beta=use_beta)
            text = "".join(ch for ch in (ocr.classification(raw) or "") if ch.isalnum())
        except Exception:  # noqa: BLE001
            continue
        last = text or last
        if len(text) >= 4:
            return text[:4]
    return last


def _captcha_variants(image_bgr: np.ndarray) -> list[np.ndarray]:
    gray = cv2.cvtColor(image_bgr, cv2.COLOR_BGR2GRAY)
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(8, 8)).apply(gray)
    thr = cv2.threshold(clahe, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)[1]
    return [
        image_bgr,
        cv2.cvtColor(clahe, cv2.COLOR_GRAY2BGR),
        cv2.cvtColor(thr, cv2.COLOR_GRAY2BGR),
        cv2.cvtColor(255 - thr, cv2.COLOR_GRAY2BGR),
    ]


def _grab(win: window.WindowInfo) -> np.ndarray:
    return screen.grab_window(window.refresh(win))


def focus_captcha_input(win: window.WindowInfo, image_bgr: np.ndarray, *, dry_run: bool = False) -> bool:
    row = anchors.find_captcha_row(image_bgr)
    if row is None:
        return False
    _label, _cap, (ix, iy) = row
    if dry_run:
        print(f"[dry-run] focus captcha input at ({ix},{iy})")
        return True
    window.activate(win)
    input_ops.click_in_window(win, ix, iy)
    return True


def refresh_captcha(win: window.WindowInfo, image_bgr: np.ndarray, *, dry_run: bool = False) -> bool:
    """Click captcha image to refresh (collapses keyboard — caller must re-focus)."""
    row = anchors.find_captcha_row(image_bgr)
    if row is None:
        return False
    _label, cap, _inp = row
    if dry_run:
        print(f"[dry-run] refresh captcha at ({cap.cx},{cap.cy})")
        return True
    window.activate(win)
    input_ops.click_in_window(win, cap.cx, cap.cy)
    return True


def dismiss_keyboard(win: window.WindowInfo, image_bgr: np.ndarray, *, dry_run: bool = False) -> bool:
    m = anchors.keyboard_done(image_bgr)
    if m is None:
        return False
    if dry_run:
        print(f"[dry-run] keyboard 完成 at ({m.x},{m.y})")
        return True
    window.activate(win)
    input_ops.click_in_window(win, m.x, m.y)
    return True


def clear_captcha_with_backspace(
    win: window.WindowInfo,
    image_bgr: np.ndarray,
    *,
    taps: int = 6,
    dry_run: bool = False,
) -> bool:
    """Clear leftover captcha chars via red delete key (bottom-right). Do NOT click the captcha image."""
    bs = anchors.keyboard_backspace(image_bgr)
    if bs is None:
        return False
    if dry_run:
        print(f"[dry-run] backspace x{taps} at ({bs.x},{bs.y})")
        return True
    window.activate(win)
    for _ in range(taps):
        input_ops.click_in_window(win, bs.x, bs.y)
        time.sleep(0.08)
    return True


def ensure_keyboard_open(win: window.WindowInfo, *, dry_run: bool = False) -> np.ndarray:
    """Focus captcha input until soft keyboard (取消) is visible."""
    img = _grab(win)
    if locate.find_text(img, "取消") or anchors.keyboard_backspace(img):
        return img
    focus_captcha_input(win, img, dry_run=dry_run)
    time.sleep(0.55)
    return _grab(win)


def type_on_soft_keyboard(
    win: window.WindowInfo,
    image_bgr: np.ndarray,
    text: str,
    *,
    dry_run: bool = False,
) -> None:
    km = build_keymap_from_ocr(image_bgr)
    text_keys = "".join(c.lower() if c.isalpha() else c for c in text)
    missing = [
        c
        for c in text_keys
        if c not in km.mapping and c.lower() not in km.mapping and c.upper() not in km.mapping
    ]
    if dry_run:
        print(f"[dry-run] type captcha {text!r} keys={text_keys!r} (missing={missing})")
        return
    if missing:
        raise RuntimeError(f"软键盘缺键 {missing}，拒绝用系统按键乱输（会输不进小程序）")
    window.activate(win)
    for x, y in km.path_for(text_keys):
        input_ops.click_in_window(win, x, y)
        time.sleep(0.15)


def input_looks_filled(image_bgr: np.ndarray) -> bool:
    """True if captcha input strip has dark ink (typed chars), not only underscores."""
    row = anchors.find_captcha_row(image_bgr)
    if row is None:
        return False
    _label, cap, (ix, iy) = row
    h, w = image_bgr.shape[:2]
    x0 = min(w - 1, cap.x1 + 4)
    x1 = min(w, max(ix + 50, cap.x1 + int(cap.w * 1.3)))
    y0 = max(0, cap.y0 - 2)
    y1 = min(h, cap.y1 + 6)
    strip = image_bgr[y0:y1, x0:x1]
    if strip.size == 0:
        return False
    gray = cv2.cvtColor(strip, cv2.COLOR_BGR2GRAY)
    # Typed glyphs are dark; empty underscores are light
    dark = int((gray < 120).sum())
    return dark >= 40


def fill_captcha_and_submit(
    win: window.WindowInfo,
    *,
    dry_run: bool = False,
    max_tries: int = 5,
) -> str:
    """Confirm-page captcha — multi-round OK, but each round is strictly once:

      1) Click input area (right of captcha image) → keyboard
      2) OCR image → type 4 chars **once**
      3) Click keyboard top-right「完成」**once**
      4) Click bottom「确定预约」**once**
      5) Wait ~2s and check:
         - success → done
         -「验证码错误」→ input was reset; start a **new** round (new OCR)
         -「请输入验证码」→ empty submit; fail this round cleanly

    Never re-type the same answer in a busy loop inside one round.
    """
    last_code = ""
    tries = max(1, min(int(max_tries), 5))

    for attempt in range(tries):
        if attempt > 0:
            print(f"[captcha] round {attempt + 1}/{tries} after previous reject…")
            time.sleep(2.0)

        img = _grab(win)
        snap = ui_state.classify(img)
        print(f"[captcha] {ui_state.snapshot_summary(snap)}")

        if snap.page == Page.SUCCESS:
            return last_code or "OK"
        if snap.page != Page.CONFIRM_ORDER:
            raise RuntimeError(f"不在确认预约页: {snap.page.value}")

        if snap.toast_need_input or snap.toast_error:
            time.sleep(1.5)
            img = _grab(win)
            snap = ui_state.classify(img)

        # --- step 1: click input once (keyboard may already be open) ---
        if not snap.keyboard_open:
            focus_captcha_input(win, img, dry_run=dry_run)
            time.sleep(0.8)
            img = _grab(win)

        # --- step 2: OCR once + type once (do NOT click captcha image) ---
        row = anchors.find_captcha_row(img)
        if row is None:
            raise RuntimeError("确认页未找到「验证码」锚点")
        _label, cap, _inp = row
        code = ocr_captcha_crop(crop_captcha_image(img, cap))
        last_code = code
        print(f"[captcha] round={attempt + 1} ocr={code!r}")
        if len(code) < 4:
            # Refresh once to get a readable image, then this round ends → next round OCRs anew
            refresh_captcha(win, img, dry_run=dry_run)
            time.sleep(1.2)
            if attempt + 1 >= tries:
                raise RuntimeError(f"验证码 OCR 不足 4 位: {code!r}")
            continue

        # After「验证码错误」input is empty; only clear if leftovers remain
        if ui_state.read_captcha_input_text(img):
            clear_captcha_with_backspace(win, img, taps=6, dry_run=dry_run)
            time.sleep(0.3)
            img = _grab(win)

        type_on_soft_keyboard(win, img, code, dry_run=dry_run)
        time.sleep(0.8)
        img = _grab(win)
        typed = ui_state.read_captcha_input_text(img)
        filled = input_looks_filled(img)
        print(f"[captcha] typed_ocr={typed!r} ink_filled={filled}")
        # Input-slot OCR is unreliable (e.g. 4jne → '4һn'); trust soft-key clicks + ink
        if not dry_run and not filled and len(typed) < 4:
            raise RuntimeError(f"软键盘输入未生效 typed={typed!r} code={code!r}")

        # --- step 3: 完成 once ---
        if not dismiss_keyboard(win, img, dry_run=dry_run):
            raise RuntimeError("未找到键盘右上角「完成」")
        time.sleep(1.2)

        img = _grab(win)
        snap = ui_state.classify(img)
        filled2 = input_looks_filled(img)
        print(f"[captcha] after完成 {ui_state.snapshot_summary(snap)} ink={filled2}")

        # Allow submit if ink still present even when OCR of slots is weak
        can = ui_state.can_submit_order(snap) or (
            snap.page == Page.CONFIRM_ORDER
            and not snap.keyboard_open
            and filled2
            and not snap.toast_error
            and not snap.toast_need_input
        )
        if not dry_run and not can:
            raise RuntimeError(
                f"不能点确定预约: phase={snap.captcha.value} input={snap.input_chars!r} ink={filled2}"
            )

        # --- step 4: 确定预约 once ---
        btn = None
        for hit in locate.ocr_image(img):
            if "确定预约" in hit.text and hit.y >= img.shape[0] * 0.7:
                btn = locate.Match(hit.x, hit.y, hit.score, label="确定预约")
                break
        if btn is None:
            btn = anchors.find_confirm_button(img)
        if btn is None:
            raise RuntimeError("未找到底部「确定预约」")

        print(f"[captcha] click 确定预约 ({btn.x},{btn.y}) code={code!r}")
        if dry_run:
            return code
        window.activate(win)
        input_ops.click_in_window(win, btn.x, btn.y)

        observed = observe_submit_feedback(win, duration_s=3.0, interval_s=0.15)
        print(
            f"[captcha] after确定 outcome={observed.outcome} "
            f"toast={observed.toast_kind} page={observed.page.value} "
            f"frames={observed.frames_checked} ev={observed.evidence}"
        )

        if observed.outcome in (SubmitOutcome.SUCCESS, SubmitOutcome.LEFT_CONFIRM):
            return code
        if observed.outcome == SubmitOutcome.CAPTCHA_EMPTY:
            raise RuntimeError(
                "toast「请输入验证码」— 输入链路故障（勿当验证码错误换图）"
            )
        if observed.outcome == SubmitOutcome.CAPTCHA_WRONG:
            print(f"toast 验证码错误 code={code!r} — 开新轮，重新识别并只输一次")
            continue
        # UNKNOWN_AFTER_SUBMIT: verify order before any blind resubmit
        verified = verify_booking_after_unknown(win)
        if verified:
            print("[captcha] UNKNOWN_AFTER_SUBMIT → 核验到成功/订单，停止重试")
            return code
        raise RuntimeError(
            "UNKNOWN_AFTER_SUBMIT：未捕获 toast 且未核验到订单 — 禁止盲点第二次确定预约"
        )

    raise RuntimeError(f"验证码未通过（最后={last_code!r}，已 {tries} 轮）")


def observe_submit_feedback(
    win: window.WindowInfo,
    *,
    duration_s: float = 3.0,
    interval_s: float = 0.15,
    grab=None,
) -> SubmitObserveResult:
    """Burst-capture after 确定预约 to catch 1–2s toasts before they vanish."""
    grab_fn = grab or (lambda w: screen.grab_window(window.refresh(w)))
    deadline = time.monotonic() + duration_s
    frames = 0
    evidence: list[str] = []
    last_page = Page.CONFIRM_ORDER
    saw_wrong = False
    saw_empty = False

    while time.monotonic() < deadline:
        img = grab_fn(win)
        frames += 1
        kind = anchors.captcha_toast_kind(img)
        snap = ui_state.classify(img)
        last_page = snap.page
        if kind == "need_input":
            saw_empty = True
            evidence.append("toast:请输入验证码")
            return SubmitObserveResult(
                SubmitOutcome.CAPTCHA_EMPTY, kind, snap.page, frames, evidence
            )
        if kind == "wrong":
            saw_wrong = True
            evidence.append("toast:验证码错误")
            # Keep sampling briefly in case success follows; but wrong toast is decisive
            return SubmitObserveResult(
                SubmitOutcome.CAPTCHA_WRONG, kind, snap.page, frames, evidence
            )
        if snap.page == Page.SUCCESS:
            evidence.append("page:success")
            return SubmitObserveResult(SubmitOutcome.SUCCESS, None, snap.page, frames, evidence)
        if snap.page == Page.MY_RESERVATIONS:
            evidence.append("page:my_reservations")
            return SubmitObserveResult(SubmitOutcome.SUCCESS, None, snap.page, frames, evidence)
        if snap.page != Page.CONFIRM_ORDER:
            evidence.append(f"page:{snap.page.value}")
            return SubmitObserveResult(
                SubmitOutcome.LEFT_CONFIRM, None, snap.page, frames, evidence
            )
        time.sleep(interval_s)

    if saw_empty:
        return SubmitObserveResult(
            SubmitOutcome.CAPTCHA_EMPTY, "need_input", last_page, frames, evidence
        )
    if saw_wrong:
        return SubmitObserveResult(
            SubmitOutcome.CAPTCHA_WRONG, "wrong", last_page, frames, evidence
        )
    evidence.append("no_toast_in_window")
    return SubmitObserveResult(
        SubmitOutcome.UNKNOWN_AFTER_SUBMIT, None, last_page, frames, evidence
    )


def verify_booking_after_unknown(win: window.WindowInfo, *, grabs: int = 4, gap_s: float = 0.6) -> bool:
    """After UNKNOWN_AFTER_SUBMIT: check success / my list / order detail — never re-click submit."""
    for _ in range(grabs):
        img = screen.grab_window(window.refresh(win))
        snap = ui_state.classify(img)
        if snap.page in (Page.SUCCESS, Page.MY_RESERVATIONS, Page.ORDER_DETAIL):
            return True
        if snap.page != Page.CONFIRM_ORDER:
            # Left confirm without toast — treat as likely success path
            return True
        time.sleep(gap_s)
    return False
