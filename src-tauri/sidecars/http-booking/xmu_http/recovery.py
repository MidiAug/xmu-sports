"""Protocol launch and verified cache handoff, without any visual automation.

Launching WeChat does not issue a fresh wx.login code. If its own runtime does
not refresh the entry, a user must confirm login and enter venue booking.
"""
from __future__ import annotations

import hashlib
import json
import os
import time
from urllib.parse import urlencode

from .app import check_auth
from .session import BookingError, discover_entry

APP_ID = "wx81a2b2fa90759cb7"


def launch_miniprogram():
    if os.name != "nt":
        raise BookingError("LAUNCH_MP_NEED_DESKTOP", "请在 Windows 电脑微信打开厦大体育")
    try:
        os.startfile("weixin://launchapplet/?" + urlencode({"app_id": APP_ID}))
    except OSError as error:
        raise BookingError("LAUNCH_MP_NEED_DESKTOP", "无法唤起厦大体育，请确认电脑微信已安装并登录") from error


def entry_digest(entry):
    # Digest stays in memory. No credentials or signed URL enter the event log.
    return hashlib.sha256(json.dumps(entry.fields, sort_keys=True).encode()).hexdigest()


def account_digest(entry):
    return hashlib.sha256(entry.uid.encode()).hexdigest()


def recover_auth(*, expected_account=None, cancelled=None, progress=None, timeout_seconds=40):
    def emit(phase, **details):
        if progress:
            progress({"phase": phase, **details})

    def check_cancel():
        if cancelled is not None and cancelled.is_set():
            raise BookingError("CANCELLED", "登录恢复已取消")

    check_cancel()
    initial = None
    try:
        initial = discover_entry()
    except BookingError as error:
        if error.code not in {"NO_ENTRY", "INVALID_ENTRY"}:
            raise
    if initial:
        account = account_digest(initial)
        if expected_account and expected_account != account:
            raise BookingError("ACCOUNT_CHANGED", "微信账号发生变化，已停止登录恢复")
        expected_account = account
    last_seen = entry_digest(initial) if initial else None
    profile = initial.profile if initial else None
    emit("launching_login", automatic_native_login=False)
    launch_miniprogram()
    started = time.monotonic()
    last_notice = -5.0
    while time.monotonic() - started < timeout_seconds:
        check_cancel()
        elapsed = time.monotonic() - started
        if elapsed - last_notice >= 5:
            emit("waiting_login_entry", elapsed_seconds=int(elapsed),
                 remaining_seconds=max(0, int(timeout_seconds - elapsed)),
                 manual_confirmation_required=True)
            last_notice = elapsed
        try:
            # Discover globally to detect multiple profiles/account changes.
            entry = discover_entry()
        except BookingError as error:
            if error.code not in {"NO_ENTRY", "INVALID_ENTRY", "SESSION_READ_FAILED"}:
                raise
        else:
            fingerprint = account_digest(entry)
            if expected_account and expected_account != fingerprint:
                raise BookingError("ACCOUNT_CHANGED", "登录账号发生变化，未继续另一账号的预约")
            if profile is not None and entry.profile != profile:
                raise BookingError("AMBIGUOUS_PROFILE", "微信配置发生变化，请重新检测登录")
            changed = entry_digest(entry)
            if changed != last_seen:
                last_seen = changed
                emit("validating_login_entry")
                check_cancel()
                result = check_auth(entry.profile)
                check_cancel()
                if result.get("account_fingerprint") != fingerprint:
                    raise BookingError("ACCOUNT_CHANGED", "验证期间登录账号发生变化")
                if result.get("ok"):
                    return {**result, "recovery_mode": "protocol_cache",
                            "fresh_entry_observed": True, "automatic_native_login": False}
                if result.get("code") not in {"SESSION_EXPIRED", "INVALID_ENTRY"}:
                    return result
                emit("login_entry_rejected", code=result.get("code"))
        if cancelled is not None:
            cancelled.wait(1)
        else:
            time.sleep(1)
    check_cancel()
    return {"ok": False, "state": "needs_login", "code": "LOGIN_CONFIRM_REQUIRED",
            "phase": "waiting_login_entry", "automatic_native_login": False,
            "fresh_entry_verified": False,
            "message": "未取得有效的新登录票据。请在电脑微信打开厦大体育，确认登录与协议、授权，再进入场馆预约后重试"}
