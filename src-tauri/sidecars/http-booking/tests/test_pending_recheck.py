from dataclasses import asdict
import hashlib
import json
from pathlib import Path
import threading
from types import SimpleNamespace

import pytest

import desktop
from xmu_http import pending
from xmu_http.session import BookingError, EntryTicket
from test_http_booking import target


@pytest.fixture
def record(tmp_path):
    reference="a"*64
    directory=tmp_path/"requests"; directory.mkdir()
    path=directory/(reference+".json")
    account=hashlib.sha256(b"1234").hexdigest()
    original={"ok":False,"state":"unknown","account_fingerprint":account,
              "target":asdict(target(court=None)),"submitted_target":asdict(target(court="04",end="21:00"))}
    path.write_text(json.dumps(original),encoding="utf-8")
    return tmp_path,reference,path,original


def fake_reader(monkeypatch, found=None, uid="1234", verified_account=True, on_read=None):
    calls=[]
    monkeypatch.setattr(pending,"discover_entry",lambda:EntryTicket({"uid":uid},Path("fake")))
    class Reader:
        def __init__(self,entry): calls.append("client")
        def login(self): calls.append("login");return {"account_checked":verified_account}
        def find_existing(self,goal):
            calls.append(goal)
            if on_read: on_read()
            return found
        http=SimpleNamespace(close=lambda:calls.append("close"))
    monkeypatch.setattr(pending,"Client",Reader)
    return calls


def test_recheck_uses_actual_snapshot_and_updates_only_original_journal(monkeypatch,record):
    root,ref,path,original=record
    match={"verified":True,"order_id":42,"date":target().date,"time":"19:30–21:00","court":"04"}
    calls=fake_reader(monkeypatch,match)
    monkeypatch.setattr(desktop,"data_root",lambda:root)
    monkeypatch.setattr(desktop,"run_candidates",lambda *a,**kw:pytest.fail("readonly recheck reached booking"))
    result=desktop.execute({"action":"recheck_pending","request_ref":ref},threading.Event())
    assert result["verified"] and result["read_only"] and not result["booking_submitted"]
    assert calls[2]==target(court="04",end="21:00")
    saved=json.loads(path.read_text(encoding="utf-8"))
    assert saved["verified"] and saved["order_id"]==42 and saved["state"]=="submitted"
    assert saved["target"]==original["target"] and saved["submitted_target"]==original["submitted_target"]
    assert len(list(path.parent.glob("*.json")))==1
    # Repeated read-only checks are harmless; neither attempt has booking code.
    assert desktop.execute({"action":"recheck_pending","request_ref":ref},threading.Event())["verified"]


def test_absent_order_never_clears_pending_or_releases_the_submission_guard(monkeypatch,record):
    root,ref,path,original=record
    fake_reader(monkeypatch)
    result=pending.recheck(root,ref,threading.Event(),lambda _:None)
    assert not result["ok"] and result["code"]=="RESULT_UNCONFIRMED" and not result["booking_submitted"]
    saved=json.loads(path.read_text(encoding="utf-8"))
    assert saved["state"]=="unknown" and saved["submitted_target"]==original["submitted_target"]
    monkeypatch.setattr(desktop,"data_root",lambda:root)
    with pytest.raises(BookingError) as error:
        desktop.guard_unknown(SimpleNamespace(find_existing=lambda _:None),target(),original["account_fingerprint"])
    assert error.value.code=="RESULT_UNCONFIRMED"


def test_different_account_rejected_before_any_http(monkeypatch,record):
    root,ref,path,original=record
    calls=fake_reader(monkeypatch,uid="other")
    before=path.read_bytes()
    with pytest.raises(BookingError) as error: pending.recheck(root,ref,threading.Event(),lambda _:None)
    assert error.value.code=="ACCOUNT_CHANGED" and calls==[] and path.read_bytes()==before


@pytest.mark.parametrize("mutation", [
    lambda value:value.pop("submitted_target"),
    lambda value:value.pop("account_fingerprint"),
    lambda value:value["submitted_target"].update(court=None),
    lambda value:value["submitted_target"].update(end=None),
    lambda value:value["submitted_target"].update(date="bad"),
    lambda value:value["submitted_target"].update(unexpected=True),
])
def test_incomplete_legacy_record_never_guesses_target_or_reads_credentials(monkeypatch,record,mutation):
    root,ref,path,original=record
    mutation(original);path.write_text(json.dumps(original),encoding="utf-8")
    monkeypatch.setattr(pending,"discover_entry",lambda:pytest.fail("incomplete snapshot reached login"))
    with pytest.raises(BookingError) as error:pending.recheck(root,ref,threading.Event(),lambda _:None)
    assert error.value.code=="SUBMISSION_SNAPSHOT_MISSING"


@pytest.mark.parametrize("extra", [{"target":asdict(target(court="01"))},{"task":{}},{"expected_account":"other"}])
def test_caller_cannot_override_original_account_or_target(monkeypatch,extra):
    monkeypatch.setattr(pending,"recheck",lambda *a:pytest.fail("overridden lookup reached journal"))
    with pytest.raises(BookingError) as error:
        desktop.execute({"action":"recheck_pending","request_ref":"a"*64,**extra},threading.Event())
    assert error.value.code=="INVALID_RECHECK"


def test_record_changed_during_read_cannot_be_overwritten(monkeypatch,record):
    root,ref,path,original=record
    changed={**original,"submitted_target":asdict(target(court="05",end="21:00"))}
    fake_reader(monkeypatch,{"verified":True,"order_id":42},on_read=lambda:path.write_text(json.dumps(changed),encoding="utf-8"))
    with pytest.raises(BookingError) as error:pending.recheck(root,ref,threading.Event(),lambda _:None)
    assert error.value.code=="SUBMISSION_SNAPSHOT_CHANGED"
    assert json.loads(path.read_text(encoding="utf-8"))==changed


def test_failed_read_never_rewrites_record(monkeypatch,record):
    root,ref,path,original=record
    def fail(): raise BookingError("HTTP_FAILED","offline")
    fake_reader(monkeypatch,on_read=fail)
    before=path.read_bytes()
    with pytest.raises(BookingError):pending.recheck(root,ref,threading.Event(),lambda _:None)
    assert path.read_bytes()==before


def test_stop_before_recheck_does_not_login_or_change_record(monkeypatch,record):
    root,ref,path,original=record
    event=threading.Event();event.set()
    monkeypatch.setattr(pending,"discover_entry",lambda:pytest.fail("cancelled lookup read credentials"))
    with pytest.raises(BookingError) as error:pending.recheck(root,ref,event,lambda _:None)
    assert error.value.code=="CANCELLED"


@pytest.mark.parametrize("reference", ["../outside","A"*64,"a"*63,None])
def test_invalid_reference_cannot_select_arbitrary_file(record,reference):
    root,_,_,_=record
    with pytest.raises(BookingError) as error:pending.load_record(root,reference)
    assert error.value.code=="INVALID_REQUEST_REF"


def test_missing_entry_preserves_original_account_for_bound_login_recovery(monkeypatch,capsys,record):
    root,ref,path,original=record
    monkeypatch.setattr(desktop,"data_root",lambda:root)
    def no_entry():raise BookingError("NO_ENTRY","cache missing")
    monkeypatch.setattr(pending,"discover_entry",no_entry)
    desktop.handle_request(json.dumps({"action":"recheck_pending","request_ref":ref}))
    result=json.loads(capsys.readouterr().out.split(desktop.PREFIX)[-1])
    assert result["code"]=="NO_ENTRY" and result["account_fingerprint"]==original["account_fingerprint"]
    assert result["target"]==original["submitted_target"]
