"""Call SetProcessDpiAwareness before screenshot / mouse APIs."""

from __future__ import annotations

import ctypes
import sys


def ensure_dpi_awareness() -> None:
    if sys.platform != "win32":
        return
    try:
        # PROCESS_PER_MONITOR_DPI_AWARE = 2
        ctypes.windll.shcore.SetProcessDpiAwareness(2)
    except Exception:
        try:
            ctypes.windll.user32.SetProcessDPIAware()
        except Exception:
            pass
