import copy
import hashlib
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

import desktop
from xmu_http.client import Client
from xmu_http.session import BookingError, EntryTicket
from xmu_http import app
from xmu_http.worker_runtime import WorkerRuntime


def before_send():
    return {"ok":False,"code":"SESSION_EXPIRED","state":"needs_login","phase":"authenticating",
            "captcha_requested":False,"flow":["local_eligibility","reading_entry","authenticating"],
            "diagnostics":{"http":{"login":{"count":1},"order_list":{"count":1}}}}


def test_explicit_auth_failure_before_send_has_bounded_positive_proof(monkeypatch,capsys):
    monkeypatch.setattr(desktop,"execute",lambda *args:before_send())
    desktop.handle_request(json.dumps({"action":"book","expected_account":"a"*64}))
    result=json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["execution_proof"]=={"version":1,"stage":"pre_submission_auth","add_order_calls":0,"captcha_calls":0,"journal_created":False}
    assert result["account_fingerprint"]=="a"*64


@pytest.mark.parametrize("endpoint", ["/index/Index/checkLogin", "/index/stadium/mySubscribe"])
@pytest.mark.parametrize("preheated", [False, True])
def test_real_rpc_auth_failure_proves_no_send_without_bypassing_consumed_worker(monkeypatch,tmp_path,capsys,endpoint,preheated):
    import requests
    entry=EntryTicket({"uid":"1234"},Path("fake"))
    monkeypatch.setattr(desktop,"discover_entry",lambda:entry)
    monkeypatch.setattr(app,"discover_entry",lambda _:entry)
    monkeypatch.setattr(desktop,"check_target",lambda _:None)
    monkeypatch.setenv("XMU_SPORTS_DATA",str(tmp_path))
    solver=SimpleNamespace(warm=lambda:None)
    monkeypatch.setattr(desktop,"CaptchaSolver",lambda:solver)
    calls=[]
    def post(session,url,**kwargs):
        path=url.removeprefix(__import__("xmu_http.client",fromlist=["API"]).API)
        assert path in {"/index/Index/checkLogin","/index/stadium/mySubscribe"},"unexpected network or submission"
        calls.append(path)
        session.cookies.set("PHPSESSID","local-test")
        body={"status":0,"info":"登录信息失效，请退出重新登录"} if path==endpoint else {"status":1,"data":[]}
        return SimpleNamespace(status_code=200,is_redirect=False,raise_for_status=lambda:None,json=lambda:body)
    monkeypatch.setattr(requests.Session,"post",post)
    payload={"action":"book","run_id":"local-test-only","expected_account":hashlib.sha256(b"1234").hexdigest(),
             "task":{"sport":"local","venue":"local","target_date":"2099-01-02","target_time_start":"10:00","target_time_end":"11:00","allow_time_fallback":True}}
    runtime=None
    if preheated:
        runtime=WorkerRuntime();runtime.binding=runtime.context(payload);runtime.solver=solver;runtime.model_load_count=1
    desktop.handle_request(json.dumps(payload),runtime)
    result=json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["code"]=="SESSION_EXPIRED" and result["phase"]=="authenticating"
    assert result["execution_proof"]["journal_created"] is False
    assert result["execution_proof"]["add_order_calls"]==0
    assert result["details"]["api"]==("login" if endpoint.endswith("checkLogin") else "order_list")
    assert len(calls)==(1 if endpoint.endswith("checkLogin") else 2)
    assert not list(tmp_path.rglob("*.json"))
    if runtime:
        assert runtime.book_consumed,"proof must not reset or reuse a consumed process"
        with pytest.raises(BookingError,match="不允许重放"):runtime.consume(payload)


@pytest.mark.parametrize("change", [
    {"code":"SUBMIT_UNKNOWN","state":"unknown"}, {"code":"HTTP_WORKER_ERROR"},
    {"code":"HTTP_FAILED"}, {"code":"ACCOUNT_CHANGED"}, {"state":"submitted"},
    {"phase":"server_precheck"}, {"captcha_requested":True},
    {"flow":["authenticating","submitting","authenticating"]}, {"request_ref":"b"*64},
    {"attempts":[{"code":"SESSION_EXPIRED"}]},
    {"diagnostics":{"http":{"submit":{"count":1}}}},
    {"diagnostics":{"http":{"captcha_image":{"count":1}}}},
])
def test_uncertain_or_later_stage_results_never_authorize_restart(change):
    value={**before_send(),**copy.deepcopy(change)}
    assert not desktop.pre_submit_auth_proof(value,{"expected_account":"a"*64},{})


def test_live_submission_marker_alone_blocks_auth_retry_proof():
    assert not desktop.pre_submit_auth_proof(before_send(),{"expected_account":"a"*64},{"request_ref":"b"*64})
    assert not desktop.pre_submit_auth_proof(before_send(),{},{}),"unbound account cannot resume"


def test_auth_rejection_logs_endpoint_alias_without_sensitive_fields():
    client=Client(EntryTicket({"uid":"1234"},Path("fake")))
    client.post=lambda *args,**kw:{"status":0,"info":"登录信息失效"}
    try:
        with pytest.raises(BookingError) as error:client.require("/index/stadium/mySubscribe",{})
        assert error.value.details=={"api":"order_list"}
    finally:client.http.close()
