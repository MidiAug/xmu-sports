"""Current user's active reservations and one-shot, verified cancellation."""
import hashlib
import re
from datetime import datetime
from urllib.parse import urlencode

from .availability import CHINA
from .client import Client, PAGES, records
from .session import BookingError, discover_entry

LIST = "/index/stadium/mySubscribe"
DETAIL = "/index/stadium/subDetails"
CANCEL = "/index/stadium/cancelOrder"


class OrdersClient(Client):
    allow_cancel = False

    def post(self, path, *args, **kwargs):
        allowed = {"/index/Index/checkLogin", LIST, DETAIL}
        if self.allow_cancel:
            allowed.add(CANCEL)
        if path not in allowed:
            raise BookingError("ORDER_ACTION_FORBIDDEN", "订单操作不允许调用预约接口")
        return super().post(path, *args, **kwargs)


def identifier(value):
    text = str(value) if value is not None else ""
    return text if re.fullmatch(r"[1-9][0-9]{0,19}", text) else ""


def own(client, row, required=False):
    if (required and row.get("uid") is None) or (row.get("uid") is not None and str(row["uid"]) != client.ticket.uid):
        raise BookingError("ACCOUNT_MISMATCH", "预约账号不一致，请刷新后重试")


def read_detail(client, order_id):
    data = client.require(DETAIL, {"order_id": order_id, "uid": client.ticket.uid, "is_admin": "false"},
                          PAGES + "order.html?" + urlencode({"id": order_id}), timeout=(3, 6))
    if not isinstance(data, dict):
        raise BookingError("ORDER_SCHEMA_CHANGED", "预约详情结构异常")
    own(client, data)
    return data


def detail_id(item):
    first, second = identifier(item.get("details_id")), identifier(item.get("id"))
    if first and second and first != second:
        return ""
    return first or second


def collection(value):
    if not isinstance(value, (list, dict)):
        raise BookingError("ORDER_SCHEMA_CHANGED", "订单列表结构异常，请到小程序核对")
    rows = list(value.values()) if isinstance(value, dict) else value
    if any(not isinstance(row, dict) for row in rows):
        raise BookingError("ORDER_SCHEMA_CHANGED", "订单列表结构异常，请到小程序核对")
    return rows


def active_rows(client):
    seen, result = set(), []
    for page in range(1, 31):
        batch = collection(client.require(LIST, {"p": str(page)}, PAGES + "mine.html", timeout=(3, 6)))
        new = 0
        for order in batch:
            own(client, order, required=True)
            oid = identifier(order.get("order_id"))
            if not oid:
                raise BookingError("ORDER_SCHEMA_CHANGED", "预约订单标识异常")
            if oid in seen:
                continue
            seen.add(oid)
            new += 1
            if str(order.get("audit_status")) != "1":
                continue
            detail = read_detail(client, oid)
            if str(detail.get("audit_status", "1")) != "1":
                continue
            for item in collection(detail.get("details")):
                if str(item.get("status", "1")) != "1":
                    continue
                times = re.findall(r"\d{2}:\d{2}", str(item.get("interval_time", "")))
                date = str(item.get("date", ""))
                try:
                    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date) or len(times) != 2 or times[1] <= times[0]:
                        raise ValueError()
                    datetime.strptime(date + " " + times[0], "%Y-%m-%d %H:%M")
                    datetime.strptime(date + " " + times[1], "%Y-%m-%d %H:%M")
                except ValueError:
                    raise BookingError("ORDER_SCHEMA_CHANGED", "预约时段不完整，请到小程序核对")
                did = detail_id(item)
                result.append({"order_id": oid, "details_id": did, "venue": str(order.get("stadium_name") or "场馆"),
                               "sport": str(order.get("project_name") or ""), "court": str(item.get("area_name") or ""),
                               "date": date, "start": times[0], "end": times[1], "can_cancel": bool(did) and str(item.get("status")) == "1"})
        if len(batch) < 10:
            return sorted(result, key=lambda row: (row["date"], row["start"], row["end"], row["order_id"], row["details_id"]))
        if not new:
            break
    raise BookingError("ORDER_LIST_INCOMPLETE", "订单分页未完整读取，请稍后刷新")


def cancel(client, oid, did):
    matching = [row for row in active_rows(client) if row["order_id"] == oid and row["details_id"] == did and row["can_cancel"]]
    if len(matching) != 1:
        raise BookingError("ORDER_NOT_CANCELLABLE", "该场次已变化或不允许取消，请刷新核对")
    client.allow_cancel = True
    uncertain = False
    try:
        client.require(CANCEL, {"order_id": oid, "school_id": "788", "order_num": "", "details_id": did},
                       PAGES + "order.html?" + urlencode({"id": oid}), timeout=(3, 8))
    except BookingError as error:
        if error.code not in {"HTTP_FAILED", "INVALID_RESPONSE", "SERVER_BUSY", "UNEXPECTED_REDIRECT"}:
            raise
        uncertain = True
    finally:
        client.allow_cancel = False
    try:
        detail = read_detail(client, oid)
        items = [item for item in records(detail.get("details")) if detail_id(item) == did]
        confirmed = str(detail.get("audit_status")) == "3" or (len(items) == 1 and str(items[0].get("status")) == "3")
        if confirmed:
            return {"ok": True, "state": "cancelled"}
    except BookingError:
        pass
    return {"ok": False, "state": "cancel_unconfirmed", "code": "CANCEL_UNCONFIRMED",
            "message": "取消结果尚未确认，请刷新或到小程序核对，勿重复取消", "response_uncertain": uncertain}


def execute(payload):
    cancellation = payload.get("action") == "cancel_order"
    oid, did = identifier(payload.get("order_id")), identifier(payload.get("details_id"))
    if cancellation and (not oid or not did or not re.fullmatch(r"[a-f0-9]{64}", str(payload.get("account_fingerprint", "")))):
        raise BookingError("INVALID_ORDER", "取消预约标识无效")
    ticket = discover_entry()
    account = hashlib.sha256(ticket.uid.encode()).hexdigest()
    if cancellation and payload["account_fingerprint"] != account:
        raise BookingError("ACCOUNT_CHANGED", "登录账号已变化，请刷新后操作")
    client = OrdersClient(ticket)
    try:
        if not client.login().get("account_checked"):
            raise BookingError("HTTP_AUTH_UNVERIFIED", "尚未完成订单账号核验")
        if cancellation:
            return cancel(client, oid, did)
        return {"ok": True, "account_fingerprint": account, "orders": active_rows(client),
                "fetched_at": datetime.now(CHINA).isoformat()}
    finally:
        client.http.close()
