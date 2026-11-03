import copy
import hashlib
import json
import threading
from pathlib import Path

import pytest

import desktop
from xmu_http.session import EntryTicket
from test_http_booking import target
from test_request_retry import booking_transport


def failure():
    return {"ok":False,"state":"failed","code":"HTTP_FAILED","details":{"error_type":"read_timeout"},
            "diagnostics":{"http":{"order_list":{"count":3,"errors":{"timeout":3}}}}}


@pytest.mark.parametrize("action", ["check", "book"])
def test_completed_rpc_provides_positive_no_send_network_proof(monkeypatch, capsys, action):
    monkeypatch.setattr(desktop, "execute", lambda *_: failure())
    desktop.handle_request(json.dumps({"action":action,"expected_account":"a"*64}))
    result = json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["execution_proof"] == {"version":1,"stage":"pre_submission_network","add_order_calls":0}
    assert result["account_fingerprint"] == "a"*64


@pytest.mark.parametrize("patch", [
    {"code":"SLOT_OCCUPIED"}, {"code":"FALLBACKS_EXHAUSTED"}, {"code":"SESSION_EXPIRED"},
    {"code":"HTTP_WORKER_ERROR"}, {"code":"SUBMIT_UNKNOWN","state":"unknown"}, {"state":"submitted"},
    {"ok":True}, {"request_ref":"b"*64}, {"details":{"error_type":"tls"}},
    {"details":{"error_type":"proxy"}}, {"details":{"error_type":"invalid_json"}},
    {"diagnostics":{"http":{"submit":{"count":1}}}},
])
def test_business_errors_or_any_send_never_get_network_restart_proof(patch):
    value = {**failure(), **copy.deepcopy(patch)}
    assert not desktop.pre_submit_network_proof(value, {"action":"book","expected_account":"a"*64}, {})


def test_marker_and_unbound_identity_block_network_proof():
    assert not desktop.pre_submit_network_proof(failure(), {"action":"book"}, {})
    assert not desktop.pre_submit_network_proof(failure(), {"action":"book","expected_account":"a"*64}, {"request_ref":"b"*64})
    assert not desktop.pre_submit_network_proof(failure(), {"action":"prepare","expected_account":"a"*64}, {})


def test_real_rpc_recovery_uses_new_cycle_key_and_fresh_checks(monkeypatch, tmp_path, capsys):
    from xmu_http.client import ADD_ORDER_PATH
    calls, _, _, options = booking_transport(monkeypatch, tmp_path, "/index/stadium/chooseVerify", failures=3)
    monkeypatch.setattr(desktop, "discover_entry", lambda: EntryTicket({"uid":"1234"}, Path("fake")))
    monkeypatch.setattr(desktop, "data_root", lambda: tmp_path)
    monkeypatch.setattr(desktop, "check_target", lambda _: None)
    monkeypatch.setattr(desktop, "CaptchaSolver", lambda: options["solver"])
    goal = target()
    account = hashlib.sha256(b"1234").hexdigest()
    payload = {"action":"book","run_id":"original-frozen-run","expected_account":account,
               "task":{"task_id":"frozen-task","sport":goal.sport,"venue":goal.venue,"target_date":goal.date,
                       "target_time_start":goal.time,"target_time_end":"21:00","prefer_court":goal.court}}
    def rpc(body):
        desktop.handle_request(json.dumps(body))
        return json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    first = rpc(payload)
    assert first["code"] == "HTTP_FAILED"
    assert first["execution_proof"]["stage"] == "pre_submission_network" and calls[ADD_ORDER_PATH] == 0
    # The host waits with a cancellable clock; this is the second, fresh RPC.
    second = rpc({**payload, "network_retry":True})
    assert second["verified"] and calls[ADD_ORDER_PATH] == 1
    journals = [json.loads(path.read_text(encoding="utf-8")) for path in (tmp_path/"requests").glob("*.json")]
    assert sorted(row["state"] for row in journals) == ["not_submitted", "submitted"]
    assert all(row["source_run_id"] == payload["run_id"] for row in journals)
    assert len({row["request_ref"] for row in journals}) == 2
    assert calls["/index/Index/checkLogin"] == 2 and calls["captcha"] == 2


def test_check_after_network_wait_rejects_changed_account(monkeypatch):
    from xmu_http.session import BookingError
    monkeypatch.setattr(desktop, "check_auth", lambda **_: {"ok":True,"account_fingerprint":"b"*64})
    with pytest.raises(BookingError) as error:
        desktop.execute({"action":"check","expected_account":"a"*64}, threading.Event())
    assert error.value.code == "ACCOUNT_CHANGED"
