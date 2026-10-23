"""Locate and focus WeChat / mini-program windows on Windows."""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class WindowInfo:
    hwnd: int
    title: str
    left: int
    top: int
    right: int
    bottom: int

    @property
    def width(self) -> int:
        return max(0, self.right - self.left)

    @property
    def height(self) -> int:
        return max(0, self.bottom - self.top)

    @property
    def bbox(self) -> tuple[int, int, int, int]:
        """mss-style: left, top, width, height."""
        return (self.left, self.top, self.width, self.height)


def _require_win32() -> None:
    if sys.platform != "win32":
        raise RuntimeError("xmu_vision window helpers require Windows")


def list_windows(title_re: str = r".*") -> list[WindowInfo]:
    _require_win32()
    import win32gui  # type: ignore

    pat = re.compile(title_re, re.I)
    out: list[WindowInfo] = []

    def _enum(hwnd: int, _: object) -> None:
        if not win32gui.IsWindowVisible(hwnd):
            return
        title = win32gui.GetWindowText(hwnd) or ""
        if not title or not pat.search(title):
            return
        left, top, right, bottom = win32gui.GetWindowRect(hwnd)
        if right - left < 80 or bottom - top < 80:
            return
        out.append(WindowInfo(hwnd, title, left, top, right, bottom))

    win32gui.EnumWindows(_enum, None)
    return out


def find_wechat(*, prefer: str | None = None) -> WindowInfo:
    """Find main WeChat window by title (微信 / Weixin / WeChat).

    Title must *start with* the product name — never bare substring match
    (avoids editor tabs like ``wechat_probe.rs``).
    """
    candidates = list_windows(r"^(微信|Weixin|WeChat)(\s|\(|（|\[|$)")
    if not candidates:
        raise RuntimeError("未找到微信窗口：请先打开并登录 PC 微信")
    if prefer:
        for w in candidates:
            if prefer in w.title:
                return w
    # Prefer larger window (main frame over tiny popups)
    return max(candidates, key=lambda w: w.width * w.height)


def find_miniprogram(*, title_hint: str = "厦大体育") -> WindowInfo | None:
    """Mini-program often has its own titled window; may be None if embedded."""
    hits = list_windows(re.escape(title_hint))
    if hits:
        return max(hits, key=lambda w: w.width * w.height)
    # Fallback: 步道乐跑 shell title seen in step screenshots
    hits = list_windows(r"步道乐跑|厦大体育")
    return max(hits, key=lambda w: w.width * w.height) if hits else None


def activate(win: WindowInfo) -> None:
    _require_win32()
    import win32con  # type: ignore
    import win32gui  # type: ignore

    hwnd = win.hwnd
    if win32gui.IsIconic(hwnd):
        win32gui.ShowWindow(hwnd, win32con.SW_RESTORE)
    try:
        win32gui.SetForegroundWindow(hwnd)
    except Exception:
        # Alt trick sometimes needed when OS blocks focus steal
        import win32api  # type: ignore

        win32api.keybd_event(0x12, 0, 0, 0)  # ALT down
        win32gui.SetForegroundWindow(hwnd)
        win32api.keybd_event(0x12, 0, 2, 0)  # ALT up


def refresh(win: WindowInfo) -> WindowInfo:
    """Re-read rect after move/resize."""
    _require_win32()
    import win32gui  # type: ignore

    left, top, right, bottom = win32gui.GetWindowRect(win.hwnd)
    title = win32gui.GetWindowText(win.hwnd) or win.title
    return WindowInfo(win.hwnd, title, left, top, right, bottom)
