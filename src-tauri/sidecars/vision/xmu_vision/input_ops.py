"""Mouse / clipboard input relative to a window."""

from __future__ import annotations

import sys
import time
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from xmu_vision.window import WindowInfo


def _require_win32() -> None:
    if sys.platform != "win32":
        raise RuntimeError("input_ops requires Windows")


def click_screen(x: int, y: int, *, settle: float = 0.15) -> None:
    _require_win32()
    import win32api  # type: ignore
    import win32con  # type: ignore

    win32api.SetCursorPos((int(x), int(y)))
    time.sleep(0.05)
    win32api.mouse_event(win32con.MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    win32api.mouse_event(win32con.MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    time.sleep(settle)


def click_in_window(win: "WindowInfo", rel_x: int, rel_y: int, *, settle: float = 0.15) -> None:
    click_screen(win.left + int(rel_x), win.top + int(rel_y), settle=settle)


def drag_screen(
    x0: int,
    y0: int,
    x1: int,
    y1: int,
    *,
    steps: int = 12,
    settle: float = 0.25,
) -> None:
    """Left-button drag from (x0,y0) to (x1,y1) in screen coords."""
    _require_win32()
    import win32api  # type: ignore
    import win32con  # type: ignore

    win32api.SetCursorPos((int(x0), int(y0)))
    time.sleep(0.05)
    win32api.mouse_event(win32con.MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0)
    time.sleep(0.05)
    n = max(2, int(steps))
    for i in range(1, n + 1):
        t = i / n
        x = int(x0 + (x1 - x0) * t)
        y = int(y0 + (y1 - y0) * t)
        win32api.SetCursorPos((x, y))
        time.sleep(0.02)
    win32api.mouse_event(win32con.MOUSEEVENTF_LEFTUP, 0, 0, 0, 0)
    time.sleep(settle)


def drag_in_window(
    win: "WindowInfo",
    rel_x0: int,
    rel_y0: int,
    rel_x1: int,
    rel_y1: int,
    *,
    steps: int = 12,
    settle: float = 0.25,
) -> None:
    drag_screen(
        win.left + int(rel_x0),
        win.top + int(rel_y0),
        win.left + int(rel_x1),
        win.top + int(rel_y1),
        steps=steps,
        settle=settle,
    )


def clipboard_set(text: str) -> None:
    _require_win32()
    import win32clipboard  # type: ignore
    import win32con  # type: ignore

    win32clipboard.OpenClipboard()
    try:
        win32clipboard.EmptyClipboard()
        win32clipboard.SetClipboardData(win32con.CF_UNICODETEXT, text)
    finally:
        win32clipboard.CloseClipboard()


def hotkey(*vk_codes: int) -> None:
    """Press key combination using virtual-key codes (e.g. CTRL=0x11, V=0x56)."""
    _require_win32()
    import win32api  # type: ignore
    import win32con  # type: ignore

    for vk in vk_codes:
        win32api.keybd_event(vk, 0, 0, 0)
    time.sleep(0.05)
    for vk in reversed(vk_codes):
        win32api.keybd_event(vk, 0, win32con.KEYEVENTF_KEYUP, 0)
    time.sleep(0.1)


def paste_text(text: str) -> None:
    clipboard_set(text)
    hotkey(0x11, 0x56)  # Ctrl+V


def type_enter() -> None:
    hotkey(0x0D)
