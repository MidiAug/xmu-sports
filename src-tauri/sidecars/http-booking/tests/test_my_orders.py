from pathlib import Path

import pytest
from xmu_http import orders
from xmu_http.session import BookingError, EntryTicket


def order(oid=42, **values):
    return {"order_id": oid, "uid":"1234", "stadium_name":"示例体育馆", "project_name":"乒乓球", "audit_status":"1", **values}


def detail(**values):
    return {"uid":"1234", "audit_status":"1", "details":[{"id":"71", "date":"2026-10-27", "interval_time":"19:30–21:00", "area_name":"02", "status":"1", **values}]}


class Fake:
    def __init__(self, pages=None, details=None, failure=None, after=None):
        self.ticket=EntryTicket({"uid":"1234"},Path("fake"))
        self.pages=pages or {"1":[order()]}
        self.details=details or {"42":detail()}
        self.calls=[];self.failure=failure;self.after=after or detail(status="3")
        self.sent=False;self.allow_cancel=False
    def require(self,path,fields,*args,**kwargs):
        self.calls.append((path,fields))
        if path==orders.LIST:return self.pages.get(fields["p"],[])
        if path==orders.DETAIL:return self.after if self.sent else self.details[fields["order_id"]]
        assert path==orders.CANCEL and self.allow_cancel
        self.sent=True
        if self.failure:raise BookingError(self.failure,"模拟上游错误")
        return []


def test_filter_sort_and_pagination():
    first=[order(i,audit_status="2") for i in range(1,11)]
    fake=Fake({"1":first,"2":[order(),order(43,audit_status="3"),order(44)]},
              {"42":detail(),"44":detail(id="72",interval_time="07:00–08:00")})
    rows=orders.active_rows(fake)
    assert [r["details_id"] for r in rows]==["72","71"]
    assert len([c for c in fake.calls if c[0]==orders.DETAIL])==2


@pytest.mark.parametrize("where",["list","detail","item"])
def test_inactive_not_displayed(where):
    fake=Fake(pages={"1":[order(audit_status="3" if where=="list" else "1")]},
              details={"42":{**detail(status="2" if where=="item" else "1"),"audit_status":"2" if where=="detail" else "1"}})
    assert not orders.active_rows(fake)


@pytest.mark.parametrize("missing",[None,"other"])
def test_wrong_or_missing_owner_rejected(missing):
    with pytest.raises(BookingError,match="账号"):
        orders.active_rows(Fake(pages={"1":[order(uid=missing)]}))


def test_duplicate_pages_not_treated_as_complete():
    batch=[order(i,audit_status="2") for i in range(1,11)]
    with pytest.raises(BookingError) as error:orders.active_rows(Fake({"1":batch,"2":batch}))
    assert error.value.code=="ORDER_LIST_INCOMPLETE"


@pytest.mark.parametrize("values",[{"id":None},{"details_id":"72"}])
def test_missing_or_conflicting_detail_id_disables_cancellation(values):
    row=orders.active_rows(Fake(details={"42":detail(**values)}))[0]
    assert not row["can_cancel"]


def test_exact_cancel_form_and_verified_once():
    fake=Fake()
    assert orders.cancel(fake,"42","71")=={"ok":True,"state":"cancelled"}
    assert [fields for path,fields in fake.calls if path==orders.CANCEL]==[
        {"order_id":"42","school_id":"788","order_num":"","details_id":"71"}]
    assert not fake.allow_cancel


@pytest.mark.parametrize("failure",["HTTP_FAILED","INVALID_RESPONSE","SERVER_BUSY"])
def test_uncertain_send_only_reads_back_never_replays(failure):
    fake=Fake(failure=failure)
    assert orders.cancel(fake,"42","71")["ok"]
    assert sum(path==orders.CANCEL for path,_ in fake.calls)==1


def test_unconfirmed_does_not_claim_success():
    fake=Fake(failure="HTTP_FAILED",after=detail())
    assert orders.cancel(fake,"42","71")["state"]=="cancel_unconfirmed"


def test_rejected_cancel_is_not_retried():
    fake=Fake(failure="API_REJECTED")
    with pytest.raises(BookingError):orders.cancel(fake,"42","71")
    assert sum(path==orders.CANCEL for path,_ in fake.calls)==1


def test_wrong_detail_does_not_send():
    fake=Fake()
    with pytest.raises(BookingError):orders.cancel(fake,"42","99")
    assert not fake.sent


def test_account_change_before_login(monkeypatch):
    monkeypatch.setattr(orders,"discover_entry",lambda:EntryTicket({"uid":"1234"},Path("fake")))
    monkeypatch.setattr(orders,"OrdersClient",lambda _:pytest.fail("must not login with changed account"))
    with pytest.raises(BookingError) as error:
        orders.execute({"action":"cancel_order","order_id":"42","details_id":"71","account_fingerprint":"a"*64})
    assert error.value.code=="ACCOUNT_CHANGED"


def test_endpoint_allowlist():
    client=orders.OrdersClient(EntryTicket({"uid":"1234"},Path("fake")))
    for path in [orders.CANCEL,"/index/Stadium/addOrder","/index/index/captcha"]:
        with pytest.raises(BookingError):client.post(path,{})
    client.http.close()


def test_worker_routes_orders_without_booking(monkeypatch):
    import desktop
    monkeypatch.setattr(orders,"execute",lambda payload:{"ok":True,"state":payload["action"]})
    for action in ["my_orders","cancel_order"]:
        assert desktop.execute({"action":action},None)["state"]==action


def test_missing_status_disables_cancel_even_with_valid_id():
    data=detail()
    del data["details"][0]["status"]
    assert not orders.active_rows(Fake(details={"42":data}))[0]["can_cancel"]


@pytest.mark.parametrize("data", [None, "unexpected", [1]])
def test_malformed_list_cannot_be_reported_as_empty(data):
    fake=Fake()
    fake.pages={"1":data}
    with pytest.raises(BookingError) as error:orders.active_rows(fake)
    assert error.value.code=="ORDER_SCHEMA_CHANGED"


def test_readonly_worker_allows_list_but_blocks_cancel(monkeypatch,capsys):
    import json
    import desktop
    monkeypatch.setattr(orders,"execute",lambda payload:{"ok":True,"state":payload["action"]})
    desktop.handle_request(json.dumps({"action":"my_orders"}),readonly=True)
    assert '"state": "my_orders"' in capsys.readouterr().out
    desktop.handle_request(json.dumps({"action":"cancel_order"}),readonly=True)
    assert 'READONLY_WORKER' in capsys.readouterr().out
