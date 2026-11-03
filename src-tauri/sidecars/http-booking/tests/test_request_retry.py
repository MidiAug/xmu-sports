"""Offline fault injection at the actual HTTP transport boundary."""
import json
import threading
from collections import Counter
from datetime import datetime, timedelta, timezone
from email.utils import format_datetime
from pathlib import Path
from types import SimpleNamespace

import pytest
import requests

from xmu_http import app, retry
from xmu_http.client import ADD_ORDER_PATH, API, Client
from xmu_http.diagnostics import Diagnostics
from xmu_http.session import BookingError, EntryTicket
from test_http_booking import interval_data, target
from test_interval_retry import make_client


SUCCESS = {"status": 1, "data": []}


@pytest.mark.parametrize("path", sorted(retry.REPEATABLE_PATHS))
@pytest.mark.parametrize("failure", [requests.Timeout, requests.ConnectionError])
def test_each_allowed_post_recovers_at_transport_boundary(path, failure):
    client, calls = make_client([failure("secret URL"), SUCCESS])
    events, exchanges = [], []
    client.retry.progress = events.append
    metrics = Diagnostics(exchange_sink=exchanges.append)
    token = metrics.enter()
    try:
        assert client.require(path, {"p": "1"}) == []
        snapshot = metrics.snapshot()
    finally:
        metrics.leave(token)
    assert len(calls) == 2 and calls[0] == calls[1]
    assert len(events) == 1 and events[0]["retry_after_seconds"] == 1
    assert [row["sequence"] for row in exchanges] == [1, 2]
    assert sum(row["count"] for row in snapshot["http"].values()) == 2
    assert "secret URL" not in json.dumps([events, exchanges, snapshot])


@pytest.mark.parametrize("status", [429, 500, 502, 503, 504])
def test_transient_http_status_can_recover(status):
    client, calls = make_client([status, SUCCESS])
    assert client.require("/index/stadium/mySubscribe", {}) == []
    assert len(calls) == 2


@pytest.mark.parametrize("failure,code", [
    (401, "SESSION_EXPIRED"), (403, "HTTP_FAILED"), (501, "SERVER_BUSY"),
    (ValueError("invalid JSON"), "HTTP_FAILED"),
    (requests.exceptions.SSLError("certificate"), "HTTP_FAILED"),
    (requests.exceptions.ProxyError("proxy configuration"), "HTTP_FAILED"),
    ({"status": 0, "info": "预约名额已满"}, "SLOT_OCCUPIED"),
    ({"status": -1, "info": "登录失效"}, "SESSION_EXPIRED"),
])
def test_permanent_or_business_errors_do_not_retry(failure, code):
    client, calls = make_client([failure])
    with pytest.raises(BookingError) as error:
        client.require("/index/stadium/mySubscribe", {})
    assert error.value.code == code and len(calls) == 1


@pytest.mark.parametrize("path", [ADD_ORDER_PATH, "/index/stadium/cancelOrder", "/unknown"])
@pytest.mark.parametrize("failure", [requests.Timeout("lost response"), 503])
def test_mutations_and_unknown_endpoints_are_never_transport_retried(path, failure):
    client, calls = make_client([failure])
    with pytest.raises(BookingError):
        client.post(path, {})
    assert len(calls) == 1


def test_exhaustion_retains_failure_and_success_does_not_reset_session_budget():
    client, calls = make_client([requests.Timeout()] * 3 + [SUCCESS] + [requests.Timeout()] * 10)
    path = "/index/stadium/mySubscribe"
    for _ in range(4):
        with pytest.raises(BookingError) as error:
            client.require(path, {})
        assert error.value.code == "HTTP_FAILED"
        if len(calls) == 3:
            assert client.require(path, {}) == []
    assert client.retry.used == 6 and len(calls) == 11


@pytest.mark.parametrize("stop_in_wait", [True, False])
def test_stop_during_backoff_prevents_next_request(stop_in_wait):
    client, calls = make_client([requests.Timeout()])
    class Stop(threading.Event):
        def wait(self, delay):
            assert delay == 1
            self.set()
            return stop_in_wait
    client.retry.cancelled = Stop()
    with pytest.raises(BookingError) as error:
        client.require("/index/stadium/mySubscribe", {})
    assert error.value.code == "CANCELLED" and len(calls) == 1


def test_retry_window_is_shared_and_bounds_the_next_request(monkeypatch):
    now = [0.0]
    monkeypatch.setattr(retry.time, "monotonic", lambda: now[0])
    timeouts = []
    client, calls = make_client([requests.Timeout(), SUCCESS, requests.Timeout()])
    original = client.http.post
    def post(*args, **kwargs):
        timeouts.append(kwargs["timeout"])
        return original(*args, **kwargs)
    client.http.post = post
    monkeypatch.setattr(retry.time, "sleep", lambda delay: now.__setitem__(0, now[0] + 59))
    assert client.require("/index/stadium/mySubscribe", {}) == []
    assert timeouts[1] == (1, 1)
    now[0] = 60
    with pytest.raises(BookingError):
        client.require("/index/stadium/mySubscribe", {})
    assert len(calls) == 3 and client.retry.used == 1


def test_lookup_deadline_cannot_be_renewed_by_retries(monkeypatch):
    now = [0.0]
    monkeypatch.setattr(retry.time, "monotonic", lambda: now[0])
    def advance(delay):
        now[0] += delay
    monkeypatch.setattr(retry.time, "sleep", advance)
    client, calls = make_client([requests.Timeout()] * 3)
    original = client.http.post
    def post(*args, **kwargs):
        now[0] += 8
        return original(*args, **kwargs)
    client.http.post = post
    with pytest.raises(BookingError):
        client.find_existing(target())
    assert len(calls) == 3 and now[0] == 27
    assert calls[0][1]["timeout"] == (3, 8)
    assert calls[2][1]["timeout"][1] == 8


def test_retry_after_seconds_and_http_date_are_respected(monkeypatch):
    assert retry.retry_after("7") == 7
    future = format_datetime(datetime.now(timezone.utc) + timedelta(seconds=30), usegmt=True)
    assert 28 <= retry.retry_after(future) <= 30
    assert retry.retry_after("bad header") is None
    delays = []
    monkeypatch.setattr(retry.time, "sleep", delays.append)
    client, calls = make_client([429, SUCCESS])
    original = client.http.post
    def post(*args, **kwargs):
        response = original(*args, **kwargs)
        response.headers["Retry-After"] = "7"
        return response
    client.http.post = post
    assert client.require("/index/stadium/mySubscribe", {}) == []
    assert delays == [7] and len(calls) == 2


def test_long_retry_after_is_not_shortened_or_slept(monkeypatch):
    delays = []
    monkeypatch.setattr(retry.time, "sleep", delays.append)
    client, calls = make_client([503])
    original = client.http.post
    def post(*args, **kwargs):
        response = original(*args, **kwargs)
        response.headers["Retry-After"] = "120"
        return response
    client.http.post = post
    with pytest.raises(BookingError) as error:
        client.require("/index/stadium/mySubscribe", {})
    assert error.value.code == "SERVER_BUSY" and len(calls) == 1 and not delays


def booking_transport(monkeypatch, tmp_path, fail_path, *, uncertain=False, create_order=True, failures=1, skip=0,
                      goal=None, order_book=None):
    """Run real client/booking/journal code with only requests.Session replaced."""
    calls = Counter()
    http = requests.sessions.Session()
    http.cookies.set("PHPSESSID", "offline")
    goal = goal or target()
    order_book = [] if order_book is None else order_book
    def post(url, **kwargs):
        path = url.removeprefix(API)
        calls[path] += 1
        if path == ADD_ORDER_PATH:
            if create_order:
                order_book.append(goal)
            if uncertain:
                raise requests.ReadTimeout("response lost after server received request")
        if path == fail_path and skip < calls[path] <= skip + failures:
            raise requests.ReadTimeout("offline injection")
        if path.endswith("getStadiumList"):
            data = {"stadium": [{"id": "1", "name": goal.venue}]}
        elif path.endswith("getStadiumDetails"):
            data = {"name": goal.venue, "user_range": "[67]", "venue": [
                {"name": goal.sport, "category_id": "8", "list": [{"venue_id": "14"}]}]}
        elif path.endswith("getInterval"):
            data = interval_data()
        elif path.endswith("mySubscribe"):
            data = [{"uid": "1234", "order_id": 42 + index, "stadium_name": booked.venue,
                     "project_name": booked.sport, "audit_status": 1} for index, booked in enumerate(order_book)]
        elif path.endswith("subDetails"):
            booked = order_book[int(kwargs["data"]["order_id"]) - 42]
            data = {"uid": "1234", "audit_status": 1, "details": [
                {"date": booked.date, "interval_time": booked.time + "–21:00", "area_name": "02"}]}
        else:
            data = {"info": {}}
        response = requests.Response()
        response.status_code = 200
        response._content = json.dumps({"status": 1, "data": data}).encode()
        response.headers["Content-Type"] = "application/json"
        return response
    def get(url, **kwargs):
        calls["captcha"] += 1
        if fail_path == "captcha" and calls["captcha"] <= failures:
            raise requests.ConnectionError("captcha download failed")
        response = requests.Response()
        response.status_code = 200
        response._content = b"\x89PNG\r\n\x1a\nlatest-image"
        return response
    http.post, http.get = post, get
    monkeypatch.setattr("xmu_http.client.requests.Session", lambda: http)
    monkeypatch.setattr(app, "discover_entry", lambda _: EntryTicket({"uid": "1234"}, Path("fake")))
    monkeypatch.setattr(app, "data_root", lambda: tmp_path)
    images = []
    solver = SimpleNamespace(warm=lambda: None, solve=lambda data: images.append(data) or "a1b2")
    events = []
    options = dict(commit=True, captcha_first=True, idempotency_key="frozen-run", solver=solver,
                   progress=events.append)
    return calls, images, events, options


@pytest.mark.parametrize("path", sorted(retry.REPEATABLE_PATHS - {"/index/stadium/subDetails"}) + ["captcha"])
def test_full_booking_recovers_then_submits_exactly_once(monkeypatch, tmp_path, path):
    calls, images, events, options = booking_transport(monkeypatch, tmp_path, path)
    result = app.run_target(target(), **options)
    assert result["ok"] and result["verified"] and calls[ADD_ORDER_PATH] == 1
    assert len(images) == 1 and images[0].endswith(b"latest-image")
    retries = [event for event in events if event.get("request_retry")]
    assert len(retries) == 1 and retries[0]["phase"] != "submitting"
    assert sum(row["phase"] == "captcha" and not row.get("request_retry") for row in events) == 1
    assert calls["/index/stadium/mySubscribe"] == (4 if path.endswith("mySubscribe") else 3)


@pytest.mark.parametrize("create_order", [True, False])
def test_unknown_send_only_reconciles_and_never_replays(monkeypatch, tmp_path, create_order):
    calls, _, _, options = booking_transport(monkeypatch, tmp_path, None,
                                            uncertain=True, create_order=create_order)
    if create_order:
        result = app.run_target(target(), **options)
        assert result["reconciled"] and result["verified"]
    else:
        with pytest.raises(BookingError) as error:
            app.run_target(target(), **options)
        assert error.value.code == "SUBMIT_UNKNOWN"
        with pytest.raises(BookingError) as replay:
            app.run_target(target(), **options)
        assert replay.value.code == "RESULT_UNCONFIRMED"
    assert calls[ADD_ORDER_PATH] == 1


def test_persistent_pre_submit_failure_never_fetches_captcha_or_submits(monkeypatch, tmp_path):
    calls, images, _, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/mySubscribe", failures=99)
    with pytest.raises(BookingError) as error:
        app.run_target(target(), **options)
    assert error.value.code == "HTTP_FAILED"
    assert calls["/index/stadium/mySubscribe"] == 3 and not calls[ADD_ORDER_PATH] and not images
    assert not list((tmp_path / "requests").glob("*.json"))


def test_reported_case_login_list_succeeds_then_dedupe_times_out_and_recovers(monkeypatch, tmp_path):
    calls, _, events, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/mySubscribe", skip=1)
    result = app.run_target(target(), **options)
    assert result["verified"] and calls[ADD_ORDER_PATH] == 1
    assert calls["/index/stadium/mySubscribe"] == 4
    assert [row["phase"] for row in events if row.get("request_retry")] == ["checking_orders"]


def test_exhausted_precheck_keeps_journal_explicitly_not_submitted(monkeypatch, tmp_path):
    calls, _, _, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/chooseVerify", failures=99)
    with pytest.raises(BookingError) as error:
        app.run_target(target(), **options)
    assert error.value.code == "HTTP_FAILED" and error.value.phase == "server_precheck"
    assert not calls[ADD_ORDER_PATH]
    journal = json.loads(next((tmp_path / "requests").glob("*.json")).read_text(encoding="utf-8"))
    assert journal["state"] == "not_submitted" and "submitted_target" not in journal


def test_accepted_booking_does_not_become_failure_when_verification_exhausts(monkeypatch, tmp_path):
    calls, _, _, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/subDetails", failures=99)
    result = app.run_target(target(), **options)
    assert result["ok"] and result["state"] == "submitted" and not result["verified"]
    assert calls[ADD_ORDER_PATH] == 1 and calls["/index/stadium/subDetails"] == 3


def test_reconciliation_rounds_share_transport_retry_budget(monkeypatch, tmp_path):
    calls, _, events, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/subDetails",
                                                uncertain=True, failures=99)
    with pytest.raises(BookingError) as error:
        app.run_target(target(), **options)
    assert error.value.code == "SUBMIT_UNKNOWN" and calls[ADD_ORDER_PATH] == 1
    assert calls["/index/stadium/subDetails"] == 11  # five reads + six shared extra requests
    assert len([row for row in events if row.get("request_retry")]) == 6
