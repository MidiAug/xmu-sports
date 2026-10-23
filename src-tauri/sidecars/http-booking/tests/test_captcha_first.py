import json
import threading
import pytest
from xmu_http import app
from xmu_http.client import PAGES
from xmu_http.session import BookingError
from test_http_booking import target, interval_data, FakeSolver
from test_flow_auth import runtime
from test_fallbacks import setup_batch

def test_captcha_and_recognition_precede_one_fresh_slot_query(monkeypatch, tmp_path):
    calls = runtime(monkeypatch, tmp_path)
    original = app.Client.captcha
    def captcha(client, ref):
        assert ref == PAGES + "confirm.html"
        return original(client, ref)
    monkeypatch.setattr(app.Client, "captcha", captcha)
    phases = []
    result = app.run_target(target(), commit=True, captcha_first=True, solver=FakeSolver(), progress=lambda event: phases.append(event["phase"]))
    assert result["code"] == "SLOT_OCCUPIED" and result["captcha_first"]
    assert calls.index("captcha") < calls.index("intervals") < calls.index("/index/stadium/chooseVerify") < calls.index("addOrder")
    assert phases.index("captcha_recognized") < phases.index("querying_slots") < phases.index("slot_ready") < phases.index("submitting")
    assert calls.count("intervals") == 1 and calls.count("login") == 1

def test_auto_selection_uses_availability_changed_during_recognition(monkeypatch, tmp_path):
    data = interval_data()
    cell = data["time_slot_list"][0]["slots"][1]
    data["time_slot_list"][0]["slots"].append({**cell, "area_name":"03", "column_id":"51"})
    calls = runtime(monkeypatch, tmp_path, data)
    class Solver(FakeSolver):
        def solve(self, image):
            cell["selected"] = cell["max_count"]
            return super().solve(image)
    sent = []
    original = app.Client.post
    def post(client, path, fields, ref):
        sent.append(fields)
        return original(client, path, fields, ref)
    monkeypatch.setattr(app.Client, "post", post)
    app.run_target(target(court=None), commit=True, captcha_first=True, solver=Solver())
    assert sent[0]["details[0][area_name]"] == "03"
    assert calls.index("captcha") < calls.index("intervals")

@pytest.mark.parametrize("code", ["SLOT_OCCUPIED", "ACCOUNT_LIMIT", "SESSION_EXPIRED"])
def test_precheck_denial_after_recognition_never_submits(monkeypatch, tmp_path, code):
    calls = runtime(monkeypatch, tmp_path, precheck=code)
    with pytest.raises(BookingError) as error:
        app.run_target(target(), commit=True, captcha_first=True, solver=FakeSolver())
    assert error.value.code == code and error.value.captcha_requested
    assert "captcha" in calls and "addOrder" not in calls
    assert json.loads(next((tmp_path / "requests").glob("*.json")).read_text(encoding="utf-8"))["state"] == "not_submitted"

def test_cancel_during_recognition_stops_before_query_or_submit(monkeypatch, tmp_path):
    calls = runtime(monkeypatch, tmp_path)
    cancelled = threading.Event()
    class Solver(FakeSolver):
        def solve(self, image):
            cancelled.set()
            return super().solve(image)
    with pytest.raises(BookingError) as error:
        app.run_target(target(), commit=True, captcha_first=True, solver=Solver(), cancelled=cancelled)
    assert error.value.code == "CANCELLED"
    assert "intervals" not in calls and "addOrder" not in calls

def test_expired_captcha_retry_reselects_in_same_session(monkeypatch, tmp_path):
    calls = runtime(monkeypatch, tmp_path)
    responses = iter([{"status":0, "info":"验证码已过期"}, {"status":0, "info":"预约名额已满"}])
    clients = []
    def post(client, *args):
        clients.append(client)
        calls.append("addOrder")
        return next(responses)
    monkeypatch.setattr(app.Client, "post", post)
    result = app.run_target(target(), commit=True, captcha_first=True, solver=FakeSolver())
    assert result["code"] == "SLOT_OCCUPIED"
    assert calls.count("captcha") == calls.count("intervals") == calls.count("addOrder") == 2
    assert clients[0] is clients[1] and calls.count("login") == 1


@pytest.mark.parametrize("info,code", [
    ("登录失效，请重新获取验证码", "SESSION_EXPIRED"),
    ("每天最多预约一次，验证码已过期", "ACCOUNT_LIMIT"),
    ("预约名额已满，验证码错误", "SLOT_OCCUPIED"),
])
def test_other_explicit_failures_take_priority_over_captcha_wording(monkeypatch, tmp_path, info, code):
    calls = runtime(monkeypatch, tmp_path)
    def post(*args):
        calls.append("addOrder")
        return {"status":0,"info":info}
    monkeypatch.setattr(app.Client,"post",post)
    result=app.run_target(target(),commit=True,captcha_first=True,solver=FakeSolver())
    assert result["code"]==code
    assert calls.count("captcha")==calls.count("addOrder")==1


def test_repeated_captcha_denial_stops_after_three_attempts(monkeypatch,tmp_path):
    calls=runtime(monkeypatch,tmp_path)
    def post(*args):
        calls.append("addOrder")
        return {"status":0,"info":"验证码错误"}
    monkeypatch.setattr(app.Client,"post",post)
    result=app.run_target(target(),commit=True,captcha_first=True,solver=FakeSolver())
    assert result["code"]=="CAPTCHA_REJECTED"
    assert calls.count("captcha")==calls.count("intervals")==calls.count("addOrder")==3
    assert calls.count("/index/stadium/chooseVerify")==3

@pytest.mark.parametrize("state", ["success", "unknown", "account_limit"])
def test_lazy_fallback_is_not_queried_after_success_or_global_failure(monkeypatch, tmp_path, state):
    def first(*args, **options):
        if state == "unknown": raise BookingError("SUBMIT_UNKNOWN", "unknown")
        if state == "account_limit": raise BookingError("ACCOUNT_LIMIT", "limit")
        return {"ok":True, "verified":True}
    setup_batch(monkeypatch, tmp_path, first)
    def forbidden(): pytest.fail("no fallback query should occur")
    if state == "unknown":
        with pytest.raises(BookingError): app.run_candidates(target(), fallbacks=forbidden, commit=True)
    else:
        app.run_candidates(target(), fallbacks=forbidden, commit=True)

def test_lazy_fallback_queries_after_definite_failure_without_replaying_primary(monkeypatch, tmp_path):
    calls = []
    def run(goal, *args, **options):
        calls.append(goal.court)
        return {"ok":goal.court == "03", "code":"SLOT_OCCUPIED" if goal.court == "02" else None}
    setup_batch(monkeypatch, tmp_path, run)
    def alternatives():
        assert calls == ["02"]
        calls.append("discover")
        return [{"court":"03"}]
    phases = []
    result = app.run_candidates(target(), fallbacks=alternatives, commit=True, idempotency_key="lazy", progress=phases.append)
    assert result["ok"] and calls == ["02", "discover", "03"]
    assert len(result["attempts"]) == 2
    assert len([event for event in phases if event["phase"] == "candidate_rejected"]) == 1
    assert [event["candidate_index"] for event in phases if event["phase"] == "selecting_candidate"] == [1]
