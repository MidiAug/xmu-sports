"""Calendar helpers for vision date picking."""

from __future__ import annotations

from datetime import date, timedelta


_WEEKDAYS = ("周一", "周二", "周三", "周四", "周五", "周六", "周日")


def target_day(offset_days: int = 1, *, today: date | None = None) -> date:
    """offset 0=today, 1=tomorrow (default for interactive / fire-day booking)."""
    base = today or date.today()
    return base + timedelta(days=offset_days)


def date_ocr_needles(offset_days: int = 1, *, today: date | None = None) -> list[str]:
    """Ordered OCR needles to find the date card to click.

    Prefer relative labels, then weekday, then M月D日 / MM月DD日.
    """
    d = target_day(offset_days, today=today)
    base = today or date.today()
    needles: list[str] = []
    if offset_days == 0:
        needles.append("今天")
    elif offset_days == 1:
        needles.append("明天")
    needles.append(_WEEKDAYS[d.weekday()])
    needles.append(f"{d.month}月{d.day}日")
    needles.append(f"{d.month:02d}月{d.day:02d}日")
    if d == base:
        if "今天" not in needles:
            needles.insert(0, "今天")
    return list(dict.fromkeys(needles))  # dedupe, keep order
