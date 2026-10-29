"""Desktop RPC worker; scheduled runs may preload a single-use solver process."""
from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import threading
import time
import traceback
from pathlib import Path

# Development only; frozen worker includes these packages itself.
if not getattr(sys, "frozen", False):
    deps = Path(__file__).resolve().parents[3] / "artifacts" / "http-booking-deps"
    if deps.is_dir():
        sys.path.insert(0, str(deps))

from xmu_http.app import check_auth, run_candidates, data_root
from xmu_http.catalog import write_json
from xmu_http.client import CaptchaSolver, Client, Target
from xmu_http.session import BookingError, discover_entry
from xmu_http.eligibility import check_target
from xmu_http.diagnostics import Diagnostics, phase_changed

PREFIX = "@@XMU_HTTP@@"
STARTED = time.monotonic()
LAST_PHASE = "reading_entry"
LAST_SUBMISSION = {}
RPC_ID = None


def emit(kind, **value):
    global LAST_PHASE, LAST_SUBMISSION
    if kind == "progress":
        LAST_PHASE = value.get("phase", LAST_PHASE)
        phase_changed(LAST_PHASE)
        if value.get("request_ref"):
            LAST_SUBMISSION = {key:value[key] for key in ("request_ref","submitted_target") if key in value}
    print(PREFIX + json.dumps({"kind": kind, "rpc_id":RPC_ID, "elapsed_ms": int((time.monotonic() - STARTED) * 1000), **value}, ensure_ascii=False), flush=True)


def guard_unknown(client, target, account, sport_day=False):
    """A new run ID must not bypass an earlier uncertain send on this venue/day."""
    for path in (data_root() / "requests").glob("*.json"):
        try:
            previous = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            raise BookingError("RESULT_UNCONFIRMED", "有未完整写入的提交记录，请先核对我的预约")
        if previous.get("account_fingerprint") not in (None, account):
            continue
        original = previous.get("target") or {}
        fields = ("sport", "date") if sport_day else ("sport", "venue", "date")
        if any(original.get(field) != getattr(target, field) for field in fields):
            continue
        uncertain = previous.get("state") in ("unknown", "started")
        accepted_unverified = previous.get("state") == "submitted" and not previous.get("verified")
        if uncertain or accepted_unverified:
            existing = client.find_existing(Target(**(previous.get("submitted_target") or original)))
            if existing:
                previous.update(ok=True, state="submitted", **existing)
                write_json(path, previous)
            else:
                raise BookingError("RESULT_UNCONFIRMED", "该场馆目标日有提交结果未确认；查单仍未找到匹配记录，已停止本次预约")


def guard_sport_day(client, target):
    from xmu_http.orders import active_rows
    from xmu_http.client import same_name
    if any(row["date"] == target.date and same_name(row["sport"], target.sport)
           for row in active_rows(client)):
        raise BookingError("SPORT_DAY_ALREADY_BOOKED", "当天这项运动已有预约，未重复提交")


def task_target(task):
    return Target(sport=task["sport"], venue=task["venue"], date=task["target_date"],
                  time=task["target_time_start"], end=task["target_time_end"],
                  court=task.get("prefer_court") or None, court_offset=0)


def ordered_fallbacks(data, target, order):
    times = set()
    for block in data.get("time_slot_list") or []:
        if str(block.get("date")) != target.date:
            continue
        # Include locked/full rows: eligibility is checked afresh for each candidate.
        import re
        values = re.findall(r"\d{2}:\d{2}", str(block.get("time_range", "")))
        if len(values) == 2 and values != [target.time, target.end]:
            times.add(tuple(values))
    earlier = sorted((value for value in times if value[0] < target.time), reverse=True)
    later = sorted(value for value in times if value[0] >= target.time)
    ordered = earlier + later if order == "up_then_down" else later + earlier
    return [{"time": start, "end": end} for start, end in ordered[:8]]


def execute(payload, cancelled, runtime=None):
    action = payload.get("action")
    if not isinstance(action, str) or action not in {"self_test", "check", "check_existing", "recheck_pending", "recover_auth", "prepare", "book", "warm", "ping", "availability", "my_orders", "cancel_order"}:
        raise BookingError("INVALID_ACTION", "未知操作，未执行登录或预约请求")
    if action in {"my_orders", "cancel_order"}:
        from xmu_http.orders import execute as orders_execute
        return orders_execute(payload)
    warm_solver = runtime.consume(payload) if runtime is not None and action == "book" else None
    if action == "availability":
        from xmu_http.availability import query
        return query(data_root(),payload.get("sport"),payload.get("venue"),payload.get("date"),payload.get("force") is True,
                     cancelled=cancelled)
    if action in {"warm","ping"}:
        if runtime is None:
            raise BookingError("PERSISTENT_WORKER_REQUIRED", "预热需要受宿主管理的持续进程")
        if action == "warm":
            emit("progress",phase="worker_model_warming")
            return runtime.warm(payload,cancelled)
        return runtime.check(payload)
    if action == "recheck_pending":
        if any(key in payload for key in ("target","task","expected_account")):
            raise BookingError("INVALID_RECHECK", "再次查单只接受原请求标识，不能覆盖账号或目标")
        from xmu_http.pending import recheck
        return recheck(data_root(),payload.get("request_ref"),cancelled,lambda value:emit("progress",**value))
    if payload.get("action") == "self_test":
        from ccl_chromium_reader import ccl_chromium_localstorage
        import requests
        solver = CaptchaSolver()
        solver.warm()
        return {"ok": True, "state": "component_ready", "network_used": False,
                "reader_ready": bool(ccl_chromium_localstorage), "ocr_ready": True,
                "login_backend": "windows_uia_host", "legacy_runtime_bridge": False,
                "screen_ocr": False, "captcha_model": "common_old.onnx",
                "captcha_probe": solver.solve(__import__("base64").b64decode(payload["captcha_probe_base64"], validate=True)) if payload.get("captcha_probe_base64") else None,
                "utf8_roundtrip": payload.get("probe_text", "")}
    if payload.get("action") == "check":
        return check_auth()
    if payload.get("action") == "check_existing":
        # A separate read-only path: cannot reach run_candidates or submit.
        target = Target(**payload["target"])
        entry = discover_entry()
        account = hashlib.sha256(entry.uid.encode()).hexdigest()
        if payload.get("expected_account") and payload["expected_account"] != account:
            raise BookingError("ACCOUNT_CHANGED", "查单账号与原提交账号不一致")
        client = Client(entry)
        try:
            auth = client.login()
            if not auth.get("account_checked"):
                raise BookingError("HTTP_AUTH_UNVERIFIED", "查单尚未完成原账号核验")
            existing = client.find_existing(target)
            return {"ok": True, "state": "order_found" if existing else "order_not_found",
                    "account_fingerprint": account, "booking_submitted": False,
                    **auth, **(existing or {"verified": False})}
        finally:
            client.http.close()
    if payload.get("action") == "recover_auth":
        # The host drives normal UI. Direct worker clients can request a manual
        # protocol/cache handoff; never silently attach the old runtime hook.
        from xmu_http.recovery import recover_auth
        return recover_auth(expected_account=payload.get("expected_account"), cancelled=cancelled,
                            progress=lambda value: emit("progress", **value))
    task = payload["task"]
    target = task_target(task)
    emit("progress", phase="local_eligibility")
    check_target(target)
    entry = discover_entry()
    account = hashlib.sha256(entry.uid.encode()).hexdigest()
    if payload.get("expected_account") and payload["expected_account"] != account:
        raise BookingError("ACCOUNT_CHANGED", "登录账号与任务检测时不一致，未执行另一账号的预约")
    fallbacks = []
    def before_commit(client, candidate):
        # Run on the same authenticated session used for captcha and submission.
        # The earlier extra client/login was immediately closed and discarded.
        guard_unknown(client, candidate, account, sport_day=bool(task.get("plan_id")))
        if task.get("plan_id"):
            guard_sport_day(client, candidate)
    if task.get("allow_time_fallback"):
        def discover_fallbacks():
            if cancelled.is_set():
                raise BookingError("CANCELLED", "任务已取消，未查询备选")
            fresh = discover_entry()
            if hashlib.sha256(fresh.uid.encode()).hexdigest() != account:
                raise BookingError("ACCOUNT_CHANGED", "查询备选时账号发生变化，未继续预约")
            fallback_client = Client(fresh)
            try:
                emit("progress", phase="discovering_fallbacks")
                fallback_client.login()
                venue = fallback_client.resolve_venue(target)
                data = fallback_client.intervals(venue, cancelled=cancelled,
                                                 progress=lambda phase: emit("progress", phase=phase))
                return ordered_fallbacks(data, target, task.get("time_fallback_order"))
            finally:
                fallback_client.http.close()
        fallbacks = discover_fallbacks
    if cancelled.is_set():
        raise BookingError("CANCELLED", "预约任务已停止")
    solver = warm_solver or CaptchaSolver()
    # Warm locally before the final requests; no captcha download happens here.
    emit("progress", phase="warming")
    solver.warm()
    return run_candidates(target, fallbacks=fallbacks, commit=payload.get("action") != "prepare",
                          captcha_first=True,
                          captcha_check=payload.get("action") == "prepare", solver=solver,
                          watch_seconds=5, eligibility=check_target, idempotency_key=payload["run_id"], expected_account=account,
                          before_commit=before_commit,
                          request_context={"source_run_id":payload["run_id"],"source_task_id":task.get("task_id")},
                          cancelled=cancelled, progress=lambda value: emit("progress", **value))


def handle_request(line, runtime=None, readonly=False):
    global LAST_PHASE, LAST_SUBMISSION, RPC_ID, STARTED
    LAST_PHASE, LAST_SUBMISSION, RPC_ID = "reading_entry", {}, None
    STARTED = time.monotonic()
    cancelled = threading.Event()
    diagnostics = Diagnostics(exchange_sink=lambda value: emit("http_exchange", **value))
    diagnostic_token = diagnostics.enter()
    watch_stop = threading.Event()
    action = None
    payload = {}
    try:
        payload = json.loads(line)
        action = payload.get("action")
        RPC_ID = payload.get("rpc_id")
        if readonly and payload.get("action") not in {"warm","ping","check","check_existing","recheck_pending","self_test","availability","my_orders"}:
            raise BookingError("READONLY_WORKER", "此诊断进程禁止预约操作")
        cancel_path = Path(payload["cancel_file"]) if payload.get("cancel_file") else None
        if cancel_path:
            if cancel_path.exists():
                cancelled.set()
            def watch():
                while not watch_stop.wait(0.1):
                    if cancel_path.exists():
                        cancelled.set()
            threading.Thread(target=watch, daemon=True).start()
        result = execute(payload, cancelled, runtime=runtime) if runtime is not None else execute(payload, cancelled)
    except BookingError as exc:
        result = {"state": "unknown" if exc.code in {"SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED"} else "failed", **exc.public()}
        if action == "recheck_pending" and exc.details and exc.details.get("original_account"):
            result.update(account_fingerprint=exc.details["original_account"],target=exc.details["target"])
        result.setdefault("phase", LAST_PHASE)
    except Exception as exc:
        after_send = LAST_PHASE in {"submitting", "submission_uncertain", "waiting_order_sync", "rechecking_orders",
                                   "order_check_retry", "order_unconfirmed", "order_accepted", "order_reconciled", "verifying_order"}
        result = {"ok": False, "state": "unknown" if after_send else "failed", "code": "SUBMIT_UNKNOWN" if after_send else "HTTP_WORKER_ERROR",
                  "phase": LAST_PHASE, "message": "请求执行组件异常；若已开始提交，请先查单",
                  "diagnostic": {"exception_type": type(exc).__name__, "frames": [
                      {"file": Path(frame.filename).name, "function": frame.name, "line": frame.lineno}
                      for frame in traceback.extract_tb(exc.__traceback__)[-6:]]}}
        if LAST_PHASE in {"verifying_order", "order_accepted", "order_reconciled"}:
            result.update(ok=True, state="submitted", verified=False, code="ACCEPTED_UNVERIFIED",
                          message="学校已接受预约，但查单阶段异常，请在我的预约手动确认；不要重复下单")
        if action == "recheck_pending":
            result.update(ok=False,state="unknown",code="ORDER_RECHECK_FAILED",
                          message="只读查单异常，原预约结果仍待确认，未提交预约")
    watch_stop.set()
    for key,value in LAST_SUBMISSION.items():
        result.setdefault(key,value)
    if runtime is not None:
        result.update(worker_preheated=runtime.solver is not None,model_load_count=runtime.model_load_count,process_id=os.getpid())
    result["diagnostics"] = diagnostics.snapshot()
    if action == "book" and pre_submit_auth_proof(result,payload,LAST_SUBMISSION):
        result["account_fingerprint"] = payload["expected_account"]
        result["execution_proof"] = {"version":1,"stage":"pre_submission_auth","add_order_calls":0,
                                     "captcha_calls":0,"journal_created":False}
    diagnostics.leave(diagnostic_token)
    emit("result", **result)
    return 0


def pre_submit_auth_proof(result, payload, submission):
    """Positive, complete RPC evidence only. Missing/crashed replies prove nothing."""
    if result.get("ok") or result.get("code") not in {"NO_ENTRY","INVALID_ENTRY","SESSION_EXPIRED"}:
        return False
    if result.get("state") in {"unknown","submitted"} or submission or result.get("request_ref") or result.get("attempts"):
        return False
    account=payload.get("expected_account")
    if not isinstance(account,str) or not re.fullmatch(r"[0-9a-f]{64}",account):
        return False
    allowed={"local_eligibility","reading_entry","authenticating","warming"}
    flow=result.get("flow") or []
    if not isinstance(flow,list) or result.get("phase") not in allowed or any(phase not in allowed for phase in flow):
        return False
    http=result.get("diagnostics",{}).get("http",{})
    return not result.get("captcha_requested",False) and not http.get("submit",{}).get("count",0) and not http.get("captcha_image",{}).get("count",0)


def main():
    if hasattr(sys.stdin, "reconfigure"):
        sys.stdin.reconfigure(encoding="utf-8")
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    persistent = "--serve" in sys.argv or "--serve-readonly" in sys.argv
    emit("ready", protocol=2)
    if not persistent:
        return handle_request(sys.stdin.readline())
    from xmu_http.worker_runtime import WorkerRuntime, watch_parent
    watch_parent()
    runtime = WorkerRuntime()
    readonly = "--serve-readonly" in sys.argv
    for line in sys.stdin:
        handle_request(line,runtime,readonly)
        # A dispatched booking consumes the process even when its result was an
        # error. The host must never reconnect and resend the same intent.
        if runtime.book_consumed:
            break
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
