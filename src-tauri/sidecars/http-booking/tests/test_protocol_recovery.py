from pathlib import Path
from types import SimpleNamespace

import pytest

from xmu_http import recovery
from xmu_http.session import BookingError, EntryTicket


def ticket(token="old", uid="1234", profile="profile"):
    return EntryTicket({"uid": uid, "token": token}, Path(profile))


def environment(monkeypatch, entries, response=None):
    clock = [0]
    calls = []
    events = []
    iterator = iter(entries)
    last = entries[-1]
    def discover():
        value = next(iterator, last)
        if isinstance(value, Exception):
            raise value
        return value
    def check(profile):
        calls.append(("check", profile))
        return response or {"ok": True, "state": "ready",
                            "account_fingerprint": recovery.account_digest(last)}
    monkeypatch.setattr(recovery, "discover_entry", discover)
    monkeypatch.setattr(recovery, "launch_miniprogram", lambda: calls.append("launch"))
    monkeypatch.setattr(recovery, "check_auth", check)
    monkeypatch.setattr(recovery.time, "monotonic", lambda: clock[0])
    monkeypatch.setattr(recovery.time, "sleep", lambda seconds: clock.__setitem__(0, clock[0] + seconds))
    return calls, events


def test_launch_success_without_refresh_is_not_login_success(monkeypatch):
    calls, events = environment(monkeypatch, [ticket()])
    result = recovery.recover_auth(timeout_seconds=3, progress=events.append)
    assert result["code"] == "LOGIN_CONFIRM_REQUIRED"
    assert result["automatic_native_login"] is False
    assert calls == ["launch"]  # No repeated server polling of stale credentials.
    assert events[-1]["phase"] == "waiting_login_entry"


def test_fresh_entry_requires_server_and_account_validation(monkeypatch):
    calls, events = environment(monkeypatch, [ticket(), ticket("new")])
    result = recovery.recover_auth(timeout_seconds=3, progress=events.append)
    assert result["ok"] and result["fresh_entry_observed"]
    assert calls == ["launch", ("check", Path("profile"))]
    assert events[-1]["phase"] == "validating_login_entry"


def test_account_change_never_reaches_server(monkeypatch):
    calls, _ = environment(monkeypatch, [ticket(), ticket("new", uid="5678")])
    with pytest.raises(BookingError, match="账号发生变化") as error:
        recovery.recover_auth(timeout_seconds=3)
    assert error.value.code == "ACCOUNT_CHANGED" and calls == ["launch"]


def test_profile_change_never_reaches_server(monkeypatch):
    calls, _ = environment(monkeypatch, [ticket(), ticket("new", profile="other")])
    with pytest.raises(BookingError) as error:
        recovery.recover_auth(timeout_seconds=3)
    assert error.value.code == "AMBIGUOUS_PROFILE" and calls == ["launch"]


def test_server_rejected_new_entry_does_not_repeat_same_ticket(monkeypatch):
    calls, events = environment(monkeypatch, [ticket(), ticket("new")],
        {"ok": False, "code": "SESSION_EXPIRED", "account_fingerprint": recovery.account_digest(ticket())})
    result = recovery.recover_auth(timeout_seconds=3, progress=events.append)
    assert result["code"] == "LOGIN_CONFIRM_REQUIRED"
    assert len(calls) == 2
    assert "login_entry_rejected" in [event["phase"] for event in events]


def test_server_error_is_not_reported_as_manual_login(monkeypatch):
    environment(monkeypatch, [ticket(), ticket("new")],
        {"ok": False, "code": "NETWORK_ERROR", "account_fingerprint": recovery.account_digest(ticket())})
    assert recovery.recover_auth(timeout_seconds=3)["code"] == "NETWORK_ERROR"


def test_cancel_before_launch(monkeypatch):
    calls, _ = environment(monkeypatch, [ticket()])
    with pytest.raises(BookingError) as error:
        recovery.recover_auth(cancelled=SimpleNamespace(is_set=lambda: True))
    assert error.value.code == "CANCELLED" and calls == []


def test_expected_account_mismatch_before_launch(monkeypatch):
    calls, _ = environment(monkeypatch, [ticket()])
    with pytest.raises(BookingError) as error:
        recovery.recover_auth(expected_account="other")
    assert error.value.code == "ACCOUNT_CHANGED" and calls == []


def test_missing_initial_entry_can_be_created(monkeypatch):
    environment(monkeypatch, [BookingError("NO_ENTRY", "missing"), ticket("new")])
    assert recovery.recover_auth(timeout_seconds=3)["ok"]
