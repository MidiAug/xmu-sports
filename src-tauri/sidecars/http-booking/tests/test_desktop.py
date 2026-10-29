import json
import io
import threading
from pathlib import Path
from types import SimpleNamespace

import pytest

import desktop
from xmu_http.client import pick_slot
from xmu_http.client import Client, Target
from xmu_http.session import EntryTicket
from xmu_http.session import BookingError
from test_http_booking import interval_data, target


def test_exchange_is_streamed_before_result_with_rpc_without_changing_phase(monkeypatch, capsys):
    from xmu_http.diagnostics import record_exchange
    def execute(payload, cancelled):
        desktop.emit("progress", phase="querying_slots")
        record_exchange("/stadium/interval/getInterval", {"venue_id":"14"},
                        response=SimpleNamespace(status_code=200, headers={}),
                        result={"status":0,"info":"参数错误"})
        assert desktop.LAST_PHASE == "querying_slots"
        raise BookingError("API_REJECTED", "参数错误")
    monkeypatch.setattr(desktop, "execute", execute)
    desktop.handle_request(json.dumps({"action":"book", "rpc_id":"test-exchange"}))
    values = [json.loads(line.removeprefix(desktop.PREFIX)) for line in capsys.readouterr().out.splitlines()]
    assert [v["kind"] for v in values] == ["progress", "http_exchange", "result"]
    assert all(v["rpc_id"] == "test-exchange" for v in values)
    assert values[1]["response"] == {"status":0,"info":"参数错误"}
    assert values[-1]["diagnostics"]["exchange_count"] == 1


@pytest.mark.parametrize("action", [None, "", "preapre", "cancel", "BOOK", 42, [], {}])
def test_unknown_action_cannot_reach_any_network_or_booking(monkeypatch, action):
    monkeypatch.setattr(desktop, "discover_entry", lambda *a: pytest.fail("unknown action read credentials"))
    monkeypatch.setattr(desktop, "run_candidates", lambda *a, **kw: pytest.fail("unknown action reached booking"))
    with pytest.raises(BookingError) as error:
        desktop.execute({"action": action}, threading.Event())
    assert error.value.code == "INVALID_ACTION"


def test_desktop_reuses_authenticated_booking_client_for_pending_guard(monkeypatch, tmp_path):
    from test_flow_auth import runtime
    from xmu_http import app
    calls = runtime(monkeypatch, tmp_path)
    monkeypatch.setattr(desktop, "check_target", lambda _: None)
    monkeypatch.setattr(desktop, "discover_entry", lambda: EntryTicket({"uid": "1234"}, Path("fake")))
    monkeypatch.setattr(desktop, "CaptchaSolver", lambda: SimpleNamespace(warm=lambda: None, solve=lambda _: "abcd"))
    guarded = []
    def guard(client, candidate, account, sport_day=False):
        assert sport_day is False
        assert calls.count("login") == 1
        assert "captcha" not in calls and "addOrder" not in calls
        guarded.append(client)
    monkeypatch.setattr(desktop, "guard_unknown", guard)
    original = app.Client.captcha
    def captcha(client, ref):
        assert guarded == [client], "guard and captcha must share the exact session"
        return original(client, ref)
    monkeypatch.setattr(app.Client, "captcha", captcha)
    desktop.execute({"action":"book", "run_id":"mock-only", "task":{
        "sport":target().sport,"venue":target().venue,"target_date":target().date,
        "target_time_start":target().time,"target_time_end":"21:00"}}, threading.Event())
    assert calls.count("login") == 1


def test_login_check_is_only_login_and_existing_orders(monkeypatch):
    from xmu_http import app
    ticket = EntryTicket({"uid": "1234"}, Path("fake"))
    calls = []
    client = Client(ticket)
    def require(path, data, *args):
        calls.append((path, data))
        if path == "/index/Index/checkLogin":
            client.http.cookies.set("PHPSESSID", "test-session")
            return {}
        if path == "/index/stadium/mySubscribe":
            return [{"uid": "1234", "id": "redacted"}]
        pytest.fail("Login check attempted a non-read-only action: " + path)
    client.require = require
    monkeypatch.setattr(app, "discover_entry", lambda _: ticket)
    monkeypatch.setattr(app, "Client", lambda _: client)
    result = desktop.execute({"action": "check"}, threading.Event())
    assert result["ok"] and result["account_checked"] and result["existing_orders_read"]
    assert result["existing_order_count"] == 1
    assert [path for path, _ in calls] == ["/index/Index/checkLogin", "/index/stadium/mySubscribe"]
    assert "redacted" not in json.dumps(result)


def test_check_existing_cannot_reach_booking_code(monkeypatch):
    seen = []
    monkeypatch.setattr(desktop, "discover_entry", lambda: EntryTicket({"uid":"1234"},Path("fake")))
    monkeypatch.setattr(desktop,"run_candidates",lambda *a,**kw:pytest.fail("read-only check reached booking"))
    monkeypatch.setattr(desktop,"Client",lambda _:SimpleNamespace(login=lambda:{"authenticated":True,"account_checked":True},
        find_existing=lambda goal:seen.append(goal) or {"verified":True,"order_id":42},http=SimpleNamespace(close=lambda:None)))
    result=desktop.execute({"action":"check_existing","target":target().__dict__},threading.Event())
    assert result["verified"] and result["state"]=="order_found" and not result["booking_submitted"]
    assert seen==[target()]


@pytest.mark.parametrize("phase,exception,code,state", [
    ("order_unconfirmed",BookingError("SUBMIT_UNKNOWN","pending"),"SUBMIT_UNKNOWN","unknown"),
    ("waiting_order_sync",RuntimeError("read interrupted"),"SUBMIT_UNKNOWN","unknown"),
    ("order_reconciled",RuntimeError("journal write interrupted"),"ACCEPTED_UNVERIFIED","submitted"),
    ("warming",RuntimeError("before send"),"HTTP_WORKER_ERROR","failed"),
])
def test_worker_preserves_uncertainty_after_a_send(monkeypatch,capsys,phase,exception,code,state):
    monkeypatch.setattr(desktop.sys,"stdin",io.StringIO("{}"))
    def execute(*args):
        desktop.emit("progress",phase=phase)
        raise exception
    monkeypatch.setattr(desktop,"execute",execute)
    assert desktop.main()==0
    result=json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["code"]==code and result["state"]==state


def test_worker_recovery_uses_protocol_cache_without_runtime_hook(monkeypatch):
    from xmu_http import recovery
    seen = {}
    def recover(**options):
        seen.update(options)
        return {"ok": False, "code": "LOGIN_CONFIRM_REQUIRED"}
    monkeypatch.setattr(recovery, "recover_auth", recover)
    result = desktop.execute({"action": "recover_auth", "expected_account": "bound"}, threading.Event())
    assert result["code"] == "LOGIN_CONFIRM_REQUIRED" and seen["expected_account"] == "bound"


def test_desktop_prepare_never_enables_commit(monkeypatch):
    monkeypatch.setattr(desktop, "check_target", lambda _: None)
    monkeypatch.setattr(desktop, "discover_entry", lambda: EntryTicket({"uid": "1234"}, Path("fake")))
    monkeypatch.setattr(desktop, "Client", lambda _: SimpleNamespace(
        login=lambda: {}, http=SimpleNamespace(close=lambda: None)))
    monkeypatch.setattr(desktop, "CaptchaSolver", lambda: SimpleNamespace(warm=lambda: None))
    monkeypatch.setattr(desktop, "guard_unknown", lambda *args: pytest.fail("prepare must not submit"))
    seen = {}
    def candidates(_, **options):
        seen.update(options)
        return {"ok": True, "state": "prepared"}
    monkeypatch.setattr(desktop, "run_candidates", candidates)
    payload = {"action": "prepare", "run_id": "test", "task": {
        "sport": "乒乓球", "venue": "翔安校区乒乓球室", "target_date": "2026-10-15",
        "target_time_start": "19:30", "target_time_end": "21:00"}}
    result = desktop.execute(payload, threading.Event())
    assert result["state"] == "prepared"
    assert seen["commit"] is False and seen["captcha_check"] is True


def test_runtime_court_offset_uses_available_cells_not_physical_columns():
    data = interval_data()
    data["time_slot_list"][0]["slots"].append({**data["time_slot_list"][0]["slots"][1],
                                            "area_name": "03", "column_id": "51"})
    assert pick_slot(data, target(court=None, court_offset=0))["area_name"] == "02"
    assert pick_slot(data, target(court=None, court_offset=1))["area_name"] == "03"
    assert pick_slot(data, target(court=None, court_offset=9))["area_name"] == "03"
    assert pick_slot(data, target(court="02", court_offset=9))["area_name"] == "02"


def test_neighbor_order_uses_all_target_day_rows_including_locked_rows():
    data = {"time_slot_list": [{"date": "2026-10-13", "time_range": time_range, "slots": [{"status":"locked"}]}
                               for time_range in ("18:00-19:30", "19:30-21:00", "21:00-22:00")]
            + [{"date":"2026-10-14","time_range":"20:00-21:00"}]}
    earlier = desktop.ordered_fallbacks(data, target(end="21:00"), "up_then_down")
    later = desktop.ordered_fallbacks(data, target(end="21:00"), "down_then_up")
    assert [item["time"] for item in earlier] == ["18:00", "21:00"]
    assert [item["time"] for item in later] == ["21:00", "18:00"]


def pending_record(tmp_path, monkeypatch, state="unknown", account="account"):
    monkeypatch.setattr(desktop, "data_root", lambda: tmp_path)
    path = tmp_path / "requests" / "earlier-run.json"
    path.parent.mkdir()
    path.write_text(json.dumps({"state":state, "target":target().__dict__,
                               "account_fingerprint":account, "verified":False}), encoding="utf-8")
    return path


@pytest.mark.parametrize("state", ["unknown", "started", "submitted"])
def test_new_run_id_cannot_bypass_pending_submit_for_same_account_venue_day(tmp_path, monkeypatch, state):
    pending_record(tmp_path, monkeypatch, state)
    client = SimpleNamespace(find_existing=lambda _: None)
    with pytest.raises(BookingError) as error:
        desktop.guard_unknown(client, target(court="03", time="21:00"), "account")
    assert error.value.code == "RESULT_UNCONFIRMED"


def test_pending_submit_is_reconciled_only_when_matching_order_is_found(tmp_path, monkeypatch):
    path = pending_record(tmp_path, monkeypatch)
    client = SimpleNamespace(find_existing=lambda _: {"verified":True, "order_id":42})
    desktop.guard_unknown(client, target(), "account")
    assert json.loads(path.read_text(encoding="utf-8"))["verified"] is True


def test_pending_auto_court_is_checked_against_the_actual_submitted_court(tmp_path, monkeypatch):
    path = pending_record(tmp_path, monkeypatch)
    previous = json.loads(path.read_text(encoding="utf-8"))
    previous["submitted_target"] = target(court="04", end="21:00").__dict__
    path.write_text(json.dumps(previous), encoding="utf-8")
    seen = []
    client = SimpleNamespace(find_existing=lambda goal: seen.append(goal) or {"verified":True,"order_id":42})
    desktop.guard_unknown(client, target(), "account")
    assert seen[0].court == "04" and seen[0].end == "21:00"


def test_other_accounts_pending_record_does_not_block_current_account(tmp_path, monkeypatch):
    pending_record(tmp_path, monkeypatch, account="other")
    client = SimpleNamespace(find_existing=lambda _: pytest.fail("must not query another account's record"))
    desktop.guard_unknown(client, target(), "account")


def test_confirmed_rejection_allows_a_new_attempt(tmp_path, monkeypatch):
    pending_record(tmp_path, monkeypatch, state="rejected")
    desktop.guard_unknown(SimpleNamespace(find_existing=lambda _: None), target(), "account")


def test_desktop_task_does_not_change_absolute_date_or_preferred_court():
    result = desktop.task_target({"sport":"乒乓球", "venue":"翔安校区乒乓球室",
        "target_date":"2026-10-13", "target_time_start":"19:30", "target_time_end":"21:00",
        "prefer_court":"02", "court_offset":2})
    assert result.date == "2026-10-13" and result.court == "02" and result.court_offset == 0


@pytest.mark.parametrize("status,code", [(401,"SESSION_EXPIRED"),(429,"RATE_LIMITED"),(503,"SERVER_BUSY")])
def test_http_auth_and_server_errors_remain_distinct(status, code):
    http = SimpleNamespace(headers={}, post=lambda *args, **kwargs: SimpleNamespace(status_code=status))
    client = Client(EntryTicket({"uid":"1234"}, Path("fake")), http=http)
    with pytest.raises(BookingError) as error:
        client.post("/index/Index/checkLogin", {})
    assert error.value.code == code


def test_normalized_catalog_label_resolves_to_actual_server_name():
    client = Client(EntryTicket({"uid":"1234"}, Path("fake")))
    real_name = "动感单车室（动感单车朋辈课堂）"
    fake_details = {"name":real_name,"user_range":[1],"venue":[{"name":"动感单车室","category_id":1,"list":[{"venue_id":2}]}]}
    client.require = lambda path, *args: {"stadium":[{"id":1,"name":real_name}]} if path.endswith("getStadiumList") else fake_details
    try:
        venue = client.resolve_venue(Target("动感单车室", "动感单车室(动感单车朋辈课堂)", "2026-10-14", "19:30"))
        assert venue.name == real_name and venue.venue_id == "2"
    finally:
        client.http.close()
