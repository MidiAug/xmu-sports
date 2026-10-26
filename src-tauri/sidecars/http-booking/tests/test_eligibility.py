from datetime import datetime
import threading
import pytest
import desktop
from xmu_http.eligibility import check_target
from xmu_http.session import BookingError
from test_http_booking import target
from test_http_booking import FakeClient, FakeSolver, plan
from xmu_http.client import submit

@pytest.mark.parametrize("date,start,now,code", [
    ("2026-10-13", "19:30", "2026-10-13T06:59:59+08:00", "NOT_OPEN"),
    ("2026-10-13", "19:30", "2026-10-13T07:00:00+08:00", None),
    ("2026-10-13", "19:30", "2026-10-13T19:30:00+08:00", "BOOKING_CLOSED"),
    ("2026-10-14", "19:30", "2026-10-13T23:59:59+08:00", None),
    ("2026-10-14", "19:30", "2026-10-14T00:00:00+08:00", "NOT_OPEN"),
    ("2026-10-14", "19:30", "2026-10-13T06:59:59+08:00", "NOT_OPEN"),
    ("2026-10-15", "19:30", "2026-10-13T18:35:00+08:00", "DATE_NOT_OPEN"),
    ("2026-10-15", "19:30", "2026-10-14T07:00:00+08:00", None),
    ("2026-10-13", "19:30", "2026-10-13T11:30:00+00:00", "BOOKING_CLOSED"),
])
def test_local_booking_boundaries(date, start, now, code):
    goal = target(date=date, time=start, end="21:00")
    if code:
        with pytest.raises(BookingError) as error:
            check_target(goal, datetime.fromisoformat(now))
        assert error.value.code == code
    else:
        check_target(goal, datetime.fromisoformat(now))

def test_invalid_immediate_goal_never_reads_login_or_requests_school(monkeypatch):
    monkeypatch.setattr(desktop, "discover_entry", lambda: pytest.fail("must not read login"))
    monkeypatch.setattr(desktop, "check_target", lambda goal: check_target(goal, datetime.fromisoformat("2026-10-13T18:35:00+08:00")))
    with pytest.raises(BookingError) as error:
        desktop.execute({"action":"book", "task":{"sport":"乒乓球", "venue":"翔安校区乒乓球室",
            "target_date":"2026-10-15", "target_time_start":"16:30", "target_time_end":"18:00"}}, threading.Event())
    assert error.value.code == "DATE_NOT_OPEN"

@pytest.mark.parametrize("expires_during_captcha", [False, True])
def test_clock_expiry_does_not_send_order_and_is_not_uncertain(tmp_path, expires_during_captcha):
    client = FakeClient([])
    clock = [datetime.fromisoformat("2026-10-13T19:29:59+08:00")]
    if expires_during_captcha:
        original = client.captcha
        def captcha(ref):
            clock[0] = datetime.fromisoformat("2026-10-13T19:30:00+08:00")
            return original(ref)
        client.captcha = captcha
    else:
        clock[0] = datetime.fromisoformat("2026-10-13T19:30:00+08:00")
    with pytest.raises(BookingError) as error:
        submit(client, target(end="21:00"), plan(), FakeSolver(), tmp_path,
               eligibility=lambda goal: check_target(goal, clock[0]))
    assert error.value.code == "BOOKING_CLOSED"
    assert client.posts == []
    assert client.images == int(expires_during_captcha)
    import json
    record = json.loads(next(tmp_path.glob("*.json")).read_text(encoding="utf-8"))
    assert record["state"] == "not_submitted"

def test_submission_progress_reports_recognition_acceptance_and_verification_without_code(tmp_path):
    client = FakeClient([{"status":1, "info":"预约成功"}])
    phases = []
    result = submit(client, target(), plan(), FakeSolver(), tmp_path, progress=phases.append)
    assert result["verified"]
    assert phases.index("captcha") < phases.index("captcha_recognized") < phases.index("submitting")
    assert phases.index("submitting") < phases.index("order_accepted") < phases.index("verifying_order")
    assert "a1b2" not in phases

def test_captcha_retry_is_visible_and_not_reported_as_order_acceptance(tmp_path):
    client = FakeClient([{"status":0, "info":"验证码错误"}, {"status":1, "info":"预约成功"}])
    phases = []
    submit(client, target(), plan(), FakeSolver(), tmp_path, progress=phases.append)
    assert phases.count("captcha_retry") == 1
    assert phases.count("submitting") == 2
    assert phases.count("order_accepted") == 1

def test_slot_taken_while_solving_captcha_is_a_definite_rejection_not_success(tmp_path):
    client = FakeClient([{"status":0, "info":"当前已选【02 2026-10-13 19:30-21:00】预约名额已满"}])
    phases = []
    result = submit(client, target(), plan(), FakeSolver(), tmp_path, progress=phases.append)
    assert result["state"] == "rejected" and result["code"] == "SLOT_OCCUPIED"
    assert not result["ok"] and len(client.posts) == 1
    assert "captcha_recognized" in phases and "order_accepted" not in phases
    assert "verifying_order" not in phases
