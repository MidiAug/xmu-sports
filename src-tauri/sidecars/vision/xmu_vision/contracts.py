"""BookingTask / RunEvent contracts + JSONL event sink (shared with Tauri)."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Literal


ExecutorKind = Literal["vision", "script", "hybrid"]
FallbackOrder = Literal["up_then_down", "down_then_up"]
OverlayKind = Literal["keyboard", "toast", "modal", "notice"] | None
ResultKind = Literal[
    "success",
    "no_slots",
    "captcha_empty",
    "captcha_wrong",
    "unknown_after_submit",
    "need_human",
    "failed",
] | None


@dataclass
class BookingTask:
    task_id: str
    sport: str
    venue: str
    target_date: str  # YYYY-MM-DD absolute
    fire_at: str  # ISO-8601
    target_time_start: str
    target_time_end: str
    executor: ExecutorKind = "vision"
    allow_time_fallback: bool = False
    time_fallback_order: FallbackOrder = "down_then_up"
    court_offset: int = 2
    max_retries: int = 30
    captcha_budget: int = 5
    success_rule: str = "order_visible_or_my_list_item"
    prefer_court: str = ""

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> BookingTask:
        order_raw = str(data.get("time_fallback_order", "down_then_up"))
        order: FallbackOrder = (
            "up_then_down" if order_raw == "up_then_down" else "down_then_up"
        )
        return cls(
            task_id=str(data["task_id"]),
            sport=str(data["sport"]),
            venue=str(data["venue"]),
            target_date=str(data["target_date"]),
            fire_at=str(data["fire_at"]),
            target_time_start=str(data["target_time_start"]),
            target_time_end=str(data["target_time_end"]),
            executor=data.get("executor", "vision"),
            allow_time_fallback=bool(data.get("allow_time_fallback", False)),
            time_fallback_order=order,
            court_offset=int(data.get("court_offset", 2)),
            max_retries=int(data.get("max_retries", 30)),
            captcha_budget=int(data.get("captcha_budget", 5)),
            success_rule=str(data.get("success_rule", "order_visible_or_my_list_item")),
            prefer_court=str(data.get("prefer_court") or ""),
        )


@dataclass
class RunEvent:
    ts: str
    task_id: str
    page_id: str | None = None
    overlay: OverlayKind = None
    action: str | None = None
    evidence: list[str] = field(default_factory=list)
    result: ResultKind = None
    artifact_paths: list[str] = field(default_factory=list)
    confidence: float | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)

    @classmethod
    def now(
        cls,
        task_id: str,
        *,
        page_id: str | None = None,
        overlay: OverlayKind = None,
        action: str | None = None,
        evidence: list[str] | None = None,
        result: ResultKind = None,
        artifact_paths: list[str] | None = None,
        confidence: float | None = None,
    ) -> RunEvent:
        return cls(
            ts=datetime.now(timezone.utc).astimezone().isoformat(timespec="milliseconds"),
            task_id=task_id,
            page_id=page_id,
            overlay=overlay,
            action=action,
            evidence=list(evidence or []),
            result=result,
            artifact_paths=list(artifact_paths or []),
            confidence=confidence,
        )


class EventSink:
    """Append-only JSONL writer for a single run."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self.path.parent.mkdir(parents=True, exist_ok=True)

    def emit(self, event: RunEvent) -> None:
        with self.path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(event.to_dict(), ensure_ascii=False) + "\n")


def default_runs_root() -> Path:
    local = Path.home() / "AppData" / "Local" / "xmu-sports" / "runs"
    return local


def run_dir(task_id: str, root: Path | None = None) -> Path:
    base = root or default_runs_root()
    d = base / task_id
    d.mkdir(parents=True, exist_ok=True)
    return d
