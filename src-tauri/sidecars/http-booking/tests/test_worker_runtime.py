import hashlib
import io
import json
import threading
from pathlib import Path

import pytest

import desktop
from xmu_http import worker_runtime
from xmu_http.session import BookingError, EntryTicket


def payload(action="warm"):
    return {"action":action,"expected_account":hashlib.sha256(b"1234").hexdigest(),"run_id":"mock-only",
            "task":{"task_id":"original","sport":"乒乓球","venue":"翔安校区乒乓球室","target_date":"2026-10-22",
                    "target_time_start":"19:30","target_time_end":"21:00","allow_time_fallback":False}}


@pytest.fixture
def runtime(monkeypatch):
    loads=[]
    class Solver:
        def __init__(self):loads.append("construct")
        def preload(self):loads.append("local_inference")
        def warm(self):pass
    monkeypatch.setattr(worker_runtime,"CaptchaSolver",Solver)
    monkeypatch.setattr(desktop,"check_target",lambda _:None)
    monkeypatch.setattr(desktop,"discover_entry",lambda:EntryTicket({"uid":"1234"},Path("fake")))
    return worker_runtime.WorkerRuntime(),loads


def test_warm_and_ping_do_no_login_or_captcha_and_keep_same_model(monkeypatch,runtime):
    state,loads=runtime
    monkeypatch.setattr(desktop,"discover_entry",lambda:pytest.fail("preheat read login credentials"))
    monkeypatch.setattr(desktop,"Client",lambda *a:pytest.fail("preheat reached HTTP"))
    one=desktop.execute(payload(),threading.Event(),state)
    two=desktop.execute(payload("ping"),threading.Event(),state)
    three=desktop.execute(payload(),threading.Event(),state)
    assert one["process_id"]==two["process_id"]==three["process_id"]
    assert one["model_load_count"]==two["model_load_count"]==three["model_load_count"]==1
    assert not one["network_used"] and not two["booking_submitted"] and one["inference_preloaded"]
    assert loads==["construct","local_inference"]


@pytest.mark.parametrize("change", ["account","target","policy","revision"])
def test_preloaded_worker_rejects_changed_frozen_context(runtime,change):
    state,_=runtime;state.warm(payload(),threading.Event())
    changed=payload("book")
    if change=="account":changed["expected_account"]="b"*64
    elif change=="target":changed["task"]["target_date"]="2026-10-23"
    elif change=="policy":changed["task"]["allow_time_fallback"]=True
    else:changed["task"]["revision"]=5
    with pytest.raises(BookingError) as error:state.consume(changed)
    assert error.value.code=="WARM_CONTEXT_CHANGED"


def test_only_one_booking_dispatch_consumes_preloaded_solver_even_if_result_unknown(monkeypatch,runtime):
    state,loads=runtime;state.warm(payload(),threading.Event())
    calls=[]
    def candidates(*args,**options):
        calls.append(options)
        raise BookingError("SUBMIT_UNKNOWN","mock response lost")
    monkeypatch.setattr(desktop,"run_candidates",candidates)
    with pytest.raises(BookingError) as error:desktop.execute(payload("book"),threading.Event(),state)
    assert error.value.code=="SUBMIT_UNKNOWN" and state.book_consumed
    with pytest.raises(BookingError) as error:desktop.execute(payload("book"),threading.Event(),state)
    assert error.value.code=="WORKER_ALREADY_CONSUMED"
    assert len(calls)==1 and calls[0]["solver"] is state.solver
    assert loads==["construct","local_inference"]


def test_cancelled_warm_does_not_load_model(runtime):
    state,loads=runtime;event=threading.Event();event.set()
    with pytest.raises(BookingError) as error:state.warm(payload(),event)
    assert error.value.code=="CANCELLED" and loads==[]


def test_failed_model_load_cannot_be_reported_ready(monkeypatch,runtime):
    state,_=runtime
    def fail(self):raise RuntimeError("model unavailable")
    monkeypatch.setattr(worker_runtime.CaptchaSolver,"preload",fail)
    with pytest.raises(RuntimeError):state.warm(payload(),threading.Event())
    assert state.solver is None and state.binding is None and state.model_load_count==0


def test_readonly_diagnostic_worker_rejects_book_before_entering_execute(monkeypatch,capsys,runtime):
    state,_=runtime;state.warm(payload(),threading.Event())
    monkeypatch.setattr(desktop,"execute",lambda *a,**kw:pytest.fail("readonly worker dispatched a booking"))
    desktop.handle_request(json.dumps(payload("book")),state,readonly=True)
    result=json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["code"]=="READONLY_WORKER" and not state.book_consumed


def test_persistent_rpc_exits_after_unknown_booking_and_never_reads_queued_replay(monkeypatch,capsys,runtime):
    _,loads=runtime
    calls=[]
    def candidates(*args,**options):
        calls.append(options["solver"])
        raise BookingError("SUBMIT_UNKNOWN","mock response lost")
    monkeypatch.setattr(desktop,"run_candidates",candidates)
    monkeypatch.setattr(worker_runtime,"watch_parent",lambda:None)
    monkeypatch.setattr(desktop.sys,"argv",["desktop.py","--serve"])
    commands=[{**payload(action),"rpc_id":str(index)} for index,action in enumerate(["warm","book","book"])]
    monkeypatch.setattr(desktop.sys,"stdin",io.StringIO("\n".join(json.dumps(value) for value in commands)+"\n"))
    assert desktop.main()==0
    messages=[json.loads(line.removeprefix(desktop.PREFIX)) for line in capsys.readouterr().out.splitlines() if line.startswith(desktop.PREFIX)]
    results=[value for value in messages if value["kind"]=="result"]
    assert [value["rpc_id"] for value in results]==["0","1"]
    assert results[1]["code"]=="SUBMIT_UNKNOWN" and len(calls)==1
    assert loads==["construct","local_inference"]
