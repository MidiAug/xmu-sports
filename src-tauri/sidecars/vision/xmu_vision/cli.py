"""CLI for vision route."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from xmu_vision.flow.open_booking import FlowConfig, OpenBookingFlow, Step


def _repo_root() -> Path:
    # src-tauri/sidecars/vision/xmu_vision/cli.py → repo root
    return Path(__file__).resolve().parents[4]


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="xmu_vision", description="PC WeChat pure-vision booking helpers")
    sub = p.add_subparsers(dest="cmd", required=True)

    until_choices = [
        s.value
        for s in (
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
        )
    ]

    ob = sub.add_parser("open-booking", help="WeChat → 厦大体育 → 选场次 → 验证码 → 确定预约")
    ob.add_argument("--app-name", default="厦大体育")
    ob.add_argument("--sport", default="乒乓球", help="运动类型（任意馆方文案；默认乒乓球仅为测试样例）")
    ob.add_argument("--venue", default="", help="场馆列表文案；空则选完运动后停止")
    ob.add_argument("--prefer-time", default="", help="场次偏好，如 19:30；空则点第一个可预约色块")
    ob.add_argument(
        "--prefer-court",
        default="",
        help="多列场地球头偏好，如 02；空则点该时段最左可约格",
    )
    ob.add_argument(
        "--court-offset",
        type=int,
        default=2,
        help="同一时段可约格左→右跳过 N 格再选（0=最左；默认 2）",
    )
    ob.add_argument(
        "--allow-time-fallback",
        action="store_true",
        help="目标时段无可用格时，按最近邻换上下时段",
    )
    ob.add_argument(
        "--time-fallback-order",
        choices=["up_then_down", "down_then_up"],
        default="down_then_up",
        help="就近换时段时同距优先：先上后下 / 先下后上",
    )
    ob.add_argument(
        "--date-offset",
        type=int,
        default=1,
        help="0=今天 1=明天（默认）",
    )
    ob.add_argument(
        "--captcha-retries",
        type=int,
        default=5,
        help="验证码整轮次数（每轮只输一次→完成→确定预约；输错可开新轮，默认5，上限5）",
    )
    ob.add_argument(
        "--task-id",
        default="",
        help="若设置则写入 RunEvent JSONL（默认 %%LOCALAPPDATA%%/xmu-sports/runs/<id>/events.jsonl）",
    )
    ob.add_argument("--events-path", type=Path, default=None, help="覆盖事件 JSONL 路径")
    ob.add_argument(
        "--pop-to-sport",
        action="store_true",
        help="预约成功后 pop_until 到运动入口（每层重新认页）",
    )
    ob.add_argument(
        "--artifact-dir",
        type=Path,
        default=_repo_root() / "artifacts" / "vision",
    )
    ob.add_argument("--template-dir", type=Path, default=None)
    ob.add_argument("--dry-run", action="store_true")
    ob.add_argument("--no-ocr", action="store_true")
    ob.add_argument("--step-delay", type=float, default=1.2)
    ob.add_argument(
        "--auto-resume",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="根据当前小程序页面自动跳到对应步骤（默认开）",
    )
    ob.add_argument(
        "--from-step",
        choices=[s.value for s in Step if s != Step.DONE],
        default=Step.ACTIVATE.value,
    )
    ob.add_argument(
        "--until",
        choices=until_choices,
        default=Step.FILL_CAPTCHA.value,
        help="跑到该步后停止（含该步）",
    )

    sub.add_parser("doctor", help="Check Windows deps / WeChat window")
    sub.add_parser("probe-wechat", help="JSON: whether PC WeChat window is visible")

    cb = sub.add_parser("cancel-booking", help="我的预约 → 已预约 → 取消预约 → 确定")
    cb.add_argument("--app-name", default="厦大体育")
    cb.add_argument("--venue-hint", default="乒乓球", help="优先取消含该文案的「已预约」行")
    cb.add_argument(
        "--artifact-dir",
        type=Path,
        default=_repo_root() / "artifacts" / "vision" / "cancel",
    )
    cb.add_argument("--dry-run", action="store_true")
    cb.add_argument("--step-delay", type=float, default=2.0)
    return p


def cmd_doctor() -> int:
    from xmu_vision.dpi import ensure_dpi_awareness
    from xmu_vision import window

    ensure_dpi_awareness()
    if sys.platform != "win32":
        print("FAIL: not Windows")
        return 1
    try:
        import win32gui  # noqa: F401
        import mss  # noqa: F401
        import cv2  # noqa: F401
        import numpy  # noqa: F401
    except ImportError as e:
        print(f"FAIL: missing dep: {e}")
        print("pip install -r src-tauri/sidecars/vision/requirements.txt")
        return 1
    try:
        w = window.find_wechat()
        print(f"OK WeChat: hwnd={w.hwnd} title={w.title!r} size={w.width}x{w.height}")
    except Exception as e:  # noqa: BLE001
        print(f"WARN WeChat: {e}")
    try:
        from rapidocr_onnxruntime import RapidOCR  # noqa: F401

        print("OK OCR: rapidocr-onnxruntime")
    except Exception:
        print("WARN OCR: not installed (pip install rapidocr-onnxruntime)")
    return 0


def cmd_probe_wechat() -> int:
    import json

    if sys.platform != "win32":
        print(json.dumps({"running": False, "title": None, "miniprogram": False, "message": "仅支持 Windows"}))
        return 0
    from xmu_vision import window

    try:
        w = window.find_wechat()
        mp = window.find_miniprogram()
        print(
            json.dumps(
                {
                    "running": True,
                    "title": w.title,
                    "miniprogram": mp is not None,
                    "message": "已检测到微信窗口",
                },
                ensure_ascii=False,
            )
        )
    except Exception as e:  # noqa: BLE001
        print(
            json.dumps(
                {
                    "running": False,
                    "title": None,
                    "miniprogram": False,
                    "message": str(e),
                },
                ensure_ascii=False,
            )
        )
    return 0


def cmd_open_booking(args: argparse.Namespace) -> int:
    tpl = args.template_dir
    if tpl is None:
        default_tpl = Path(__file__).resolve().parents[1] / "assets" / "templates"
        tpl = default_tpl if default_tpl.is_dir() else None
    cfg = FlowConfig(
        app_name=args.app_name,
        sport=args.sport,
        venue=args.venue,
        prefer_time=args.prefer_time,
        prefer_court=args.prefer_court,
        court_offset=int(getattr(args, "court_offset", 2)),
        allow_time_fallback=bool(getattr(args, "allow_time_fallback", False)),
        time_fallback_order=str(getattr(args, "time_fallback_order", "down_then_up")),
        date_offset=args.date_offset,
        captcha_retries=args.captcha_retries,
        artifact_dir=args.artifact_dir,
        dry_run=args.dry_run,
        step_delay=args.step_delay,
        template_dir=tpl,
        ocr=not args.no_ocr,
        from_step=Step(args.from_step),
        until_step=Step(args.until),
        auto_resume=args.auto_resume,
        task_id=args.task_id or "",
        events_path=args.events_path,
        pop_to_sport_list=bool(getattr(args, "pop_to_sport", False)),
    )
    result = OpenBookingFlow(cfg).run()
    print(f"{'OK' if result.ok else 'FAIL'} stopped={result.stopped_at.value} {result.message}")
    for a in result.artifacts:
        print(f"  artifact: {a}")
    return 0 if result.ok else 2


def cmd_cancel_booking(args: argparse.Namespace) -> int:
    from xmu_vision.flow.cancel_booking import CancelBookingFlow, CancelConfig

    cfg = CancelConfig(
        app_name=args.app_name,
        venue_hint=args.venue_hint,
        artifact_dir=args.artifact_dir,
        dry_run=args.dry_run,
        step_delay=args.step_delay,
    )
    result = CancelBookingFlow(cfg).run()
    print(f"{'OK' if result.ok else 'FAIL'} stopped={result.stopped_at.value} {result.message}")
    for a in result.artifacts:
        print(f"  artifact: {a}")
    return 0 if result.ok else 2


def main(argv: list[str] | None = None) -> int:
    argv = argv if argv is not None else sys.argv[1:]
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.cmd == "doctor":
        return cmd_doctor()
    if args.cmd == "probe-wechat":
        return cmd_probe_wechat()
    if args.cmd == "open-booking":
        return cmd_open_booking(args)
    if args.cmd == "cancel-booking":
        return cmd_cancel_booking(args)
    parser.error(f"unknown cmd {args.cmd}")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
