import asyncio
import json
from pathlib import Path
import zlib
from types import SimpleNamespace

import pytest

from xmu_http import runtime_entry as runtime
from xmu_http import runtime_probe as probe
from xmu_http import runtime_wire as wire
from xmu_http.session import BookingError, EntryTicket


def ticket(token="old", uid="1234"):
    return EntryTicket({"uid": uid, "token": token}, Path("profile"))


def environment(monkeypatch, *, changed_account=False, reject=False, closed=True):
    calls, progress = [], []
    state = {"stage": 0, "venue": False, "clock": 0}
    class Channel:
        async def start(self): calls.append("start")
        async def close(self): calls.append("detach")
        async def restore(self): pass
        async def reenter(self, _): calls.append("reenter")
        async def inspect(self):
            label = ["立即登录", "授权登录", "场馆预约"][state["stage"]]
            return [{"context": 1, "root": True, "visible": True, "labels": [], "back": False},
                    {"context": 2 + state["stage"], "visible": True, "labels": [label], "agreement": "false"}]
        async def click(self, _, label):
            calls.append(label)
            if label in {"立即登录", "授权登录"}: state["stage"] += 1
            if label == "场馆预约": state["venue"] = True
            return True
    async def sleep(seconds): state["clock"] += seconds
    monkeypatch.setattr(runtime, "RuntimeChannel", Channel)
    monkeypatch.setattr(runtime.asyncio, "sleep", sleep)
    monkeypatch.setattr(runtime.time, "monotonic", lambda: state["clock"])
    monkeypatch.setattr(runtime, "launch_miniprogram", lambda: calls.append("launch"))
    monkeypatch.setattr(runtime, "close_xmu", lambda: calls.append("close_window"))
    monkeypatch.setattr(runtime, "activate_xmu", lambda: True)
    monkeypatch.setattr(runtime, "xmu_windows", lambda: [] if closed else [{"visible": True}])
    monkeypatch.setattr(runtime, "discover_entry", lambda: ticket("new", "5678" if changed_account else "1234") if state["venue"] else ticket())
    def auth(_):
        calls.append("server_check")
        return {"ok": not reject, "code": "SESSION_EXPIRED" if reject else None,
                "account_fingerprint": runtime.account_digest(ticket())}
    monkeypatch.setattr(runtime, "check_auth", auth)
    return calls, progress


def test_logged_out_closed_mini_program_runs_all_auth_steps_and_closes(monkeypatch):
    calls, progress = environment(monkeypatch)
    result = runtime.refresh_entry(progress=progress.append)
    assert result["ok"] and result["fresh_entry_observed"] and result["mini_program_closed"]
    assert result["native_login_performed"]
    assert calls == ["start", "launch", "reenter", "立即登录", "agreement", "授权登录",
                     "场馆预约", "server_check", "close_window", "detach"]
    assert all("token" not in event for event in progress)


def test_failure_exports_compatibility_without_credentials(monkeypatch):
    _, progress = environment(monkeypatch, reject=True)
    original = runtime.RuntimeChannel
    class DiagnosedChannel(original):
        diagnostics = {"runtime_folder": "25560", "runtime_sha256": "abc", "runtime_supported": True}
    monkeypatch.setattr(runtime, "RuntimeChannel", DiagnosedChannel)
    with pytest.raises(BookingError):
        runtime.refresh_entry(progress=progress.append)
    final = progress[-1]
    assert final["phase"] == "closing_miniprogram"
    assert final["diagnostics"]["runtime_folder"] == "25560"
    assert final["native_login_performed"] and final["venue_entered"]
    assert '"token"' not in json.dumps(progress)
    assert "profile" not in json.dumps(progress)


def test_unknown_runtime_is_reported_before_attach(monkeypatch, tmp_path):
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (None, [], {}))
    folder = tmp_path / "25561"
    folder.mkdir()
    (folder / "flue.dll").write_bytes(b"unsupported runtime")
    device = SimpleNamespace(attach=lambda _: pytest.fail("must not attach to unknown DLL"))
    process = SimpleNamespace(parameters={"path": str(folder / "WeChatAppEx.exe")}, pid=123)
    monkeypatch.setattr(runtime, "runtime_process", lambda: (device, process))
    channel = runtime.RuntimeChannel()
    with pytest.raises(BookingError) as error:
        asyncio.run(channel.start())
    assert error.value.code == "RUNTIME_UNSUPPORTED"
    compatibility = error.value.public()["details"]["compatibility"]
    assert compatibility["runtime_supported"] is False
    assert compatibility["runtime_folder"] == "25561"
    assert len(compatibility["runtime_sha256"]) == 64
    assert str(tmp_path) not in json.dumps(compatibility)


def test_changed_account_stops_before_server_and_still_cleans_up(monkeypatch):
    calls, _ = environment(monkeypatch, changed_account=True)
    with pytest.raises(BookingError) as error: runtime.refresh_entry()
    assert error.value.code == "ACCOUNT_CHANGED"
    assert "server_check" not in calls
    assert calls[-2:] == ["close_window", "detach"]


def test_server_rejection_is_not_success(monkeypatch):
    calls, _ = environment(monkeypatch, reject=True)
    with pytest.raises(BookingError) as error: runtime.refresh_entry()
    assert error.value.code == "SESSION_EXPIRED" and calls[-1] == "detach"


def test_close_failure_is_reported_after_validation(monkeypatch):
    calls, _ = environment(monkeypatch, closed=False)
    with pytest.raises(BookingError) as error: runtime.refresh_entry()
    assert error.value.code == "RUNTIME_CLOSE_FAILED"
    assert calls[-1] == "detach"


def test_wrong_initial_account_never_opens_mini_program(monkeypatch):
    calls, _ = environment(monkeypatch)
    with pytest.raises(BookingError) as error: runtime.refresh_entry(expected_account="other")
    assert error.value.code == "ACCOUNT_CHANGED" and not calls


def test_missing_kernel_is_booted_before_attaching_and_running_auth(monkeypatch):
    calls, progress = environment(monkeypatch)
    original = runtime.RuntimeChannel
    class ColdChannel(original):
        host_missing = True
        attempts = 0
        async def start(self):
            self.attempts += 1
            if self.attempts == 1:
                raise BookingError("RUNTIME_UNAVAILABLE", "not started")
            await super().start()
    monkeypatch.setattr(runtime, "RuntimeChannel", ColdChannel)
    monkeypatch.setattr(runtime, "runtime_process", lambda: (None, object()))
    result = runtime.refresh_entry(progress=progress.append)
    assert result["ok"] and result["mini_program_closed"]
    assert calls[:4] == ["launch", "close_window", "start", "launch"]
    assert "launching_runtime" in [event["phase"] for event in progress]


def test_unavailable_kernel_stops_with_bounded_wait_and_closes_boot_window(monkeypatch):
    calls, _ = environment(monkeypatch)
    class MissingChannel(runtime.RuntimeChannel):
        host_missing = True
        async def start(self): raise BookingError("RUNTIME_UNAVAILABLE", "not started")
    monkeypatch.setattr(runtime, "RuntimeChannel", MissingChannel)
    monkeypatch.setattr(runtime, "runtime_process", lambda: (None, None))
    with pytest.raises(BookingError) as error: runtime.refresh_entry(timeout_seconds=5)
    assert error.value.code == "RUNTIME_UNAVAILABLE"
    assert calls == ["launch", "close_window", "detach"]
    assert "server_check" not in calls


def test_cancellation_never_logs_in_and_closes_owned_window(monkeypatch):
    calls, _ = environment(monkeypatch)
    class Cancelled:
        def is_set(self): return True
    with pytest.raises(BookingError) as error: runtime.refresh_entry(cancelled=Cancelled())
    assert error.value.code == "CANCELLED"
    assert "launch" not in calls and calls[-1] == "detach"


def test_shutdown_does_not_restore_native_pointers_after_window_has_closed(monkeypatch):
    calls = []
    channel = runtime.RuntimeChannel()
    channel.script = SimpleNamespace(
        exports_sync=SimpleNamespace(restore=lambda: calls.append("restore")),
        unload=lambda: calls.append("unload"))
    channel.session = SimpleNamespace(detach=lambda: calls.append("detach"))
    async def sleep(_): pass
    monkeypatch.setattr(runtime.asyncio, "sleep", sleep)
    async def shutdown():
        await channel.restore()
        calls.append("window_closed")
        await channel.close()
    asyncio.run(shutdown())
    assert calls == ["restore", "window_closed", "unload", "detach"]


@pytest.mark.parametrize("show_authorize_again", [False, True])
@pytest.mark.parametrize("context_changed", [False, True])
def test_pending_login_is_not_interrupted_or_authorized_twice(monkeypatch, show_authorize_again, context_changed):
    calls, progress = environment(monkeypatch)
    class PendingChannel(runtime.RuntimeChannel):
        pending = 0
        async def inspect(self):
            states = await super().inspect()
            if "授权登录" in calls and self.pending < 3:
                self.pending += 1
                states[0]["back"] = True
                states[1] = {"context": 100 + self.pending if context_changed else 3, "visible": True,
                             "labels": ["授权登录"] if show_authorize_again else [], "agreement": "true"}
            return states
    monkeypatch.setattr(runtime, "RuntimeChannel", PendingChannel)
    result = runtime.refresh_entry(progress=progress.append)
    assert result["ok"] and calls.count("授权登录") == 1
    assert "back" not in calls
    assert "native_login_pending" in [event["phase"] for event in progress]


def test_fullscreen_focus_block_stops_without_attempting_login(monkeypatch):
    calls, progress = environment(monkeypatch)
    monkeypatch.setattr(runtime, "activate_xmu", lambda: False)
    with pytest.raises(BookingError) as error: runtime.refresh_entry(timeout_seconds=6, progress=progress.append)
    assert error.value.code == "LOGIN_FOCUS_REQUIRED"
    assert "立即登录" not in calls and "server_check" not in calls
    assert "waiting_miniprogram_focus" in [event["phase"] for event in progress]


def test_reentry_and_page_inspection_disconnects_resume_without_replaying_reentry(monkeypatch):
    calls, progress = environment(monkeypatch)
    class Reconnecting(runtime.RuntimeChannel):
        interrupted = False
        async def reenter(self, context):
            await super().reenter(context)
            raise runtime.RuntimeDisconnected("reply lost during reentry")
        async def inspect(self):
            if "reenter" in calls and not self.interrupted:
                self.interrupted = True
                raise runtime.RuntimeDisconnected("new page still connecting")
            return await super().inspect()
    monkeypatch.setattr(runtime, "RuntimeChannel", Reconnecting)
    result = runtime.refresh_entry(progress=progress.append)
    assert result["ok"] and result["fresh_entry_observed"] and result["mini_program_closed"]
    assert calls.count("reenter") == 1 and calls.count("授权登录") == 1
    assert "runtime_reconnecting" in [event["phase"] for event in progress]


def test_unexpected_error_keeps_failure_phase_and_safe_call_locations(monkeypatch):
    _, progress = environment(monkeypatch)
    class Broken(runtime.RuntimeChannel):
        async def reenter(self, _): raise RuntimeError("dummy-private-payload")
    monkeypatch.setattr(runtime, "RuntimeChannel", Broken)
    with pytest.raises(BookingError) as error:
        runtime.refresh_entry(progress=progress.append)
    details = error.value.public()["details"]
    assert details["failure_phase"] == "runtime_reenter"
    assert progress[-1]["phase"] == "closing_miniprogram"
    assert details["exception_type"] == "RuntimeError"
    assert details["frames"] and all(set(frame) == {"file", "function", "line"} for frame in details["frames"])
    assert all("/" not in frame["file"] and "\\" not in frame["file"] for frame in details["frames"])
    assert "dummy-private-payload" not in json.dumps(error.value.public())


@pytest.mark.parametrize("error_type", [runtime.RuntimeDisconnected, asyncio.TimeoutError, ValueError])
def test_authorization_reply_lost_on_disconnect_is_not_immediately_replayed(monkeypatch, error_type):
    calls, _ = environment(monkeypatch)
    class LostReply(runtime.RuntimeChannel):
        async def click(self, context, label):
            result = await super().click(context, label)
            if label == "授权登录":
                raise error_type("authorization already reached the page")
            return result
    monkeypatch.setattr(runtime, "RuntimeChannel", LostReply)
    assert runtime.refresh_entry()["ok"]
    assert calls.count("授权登录") == 1


@pytest.mark.parametrize("cancelled", [False, True])
def test_permanent_disconnect_is_bounded_and_cancellable(monkeypatch, cancelled):
    calls, _ = environment(monkeypatch)
    class Offline(runtime.RuntimeChannel):
        async def inspect(self):
            if "reenter" in calls:
                calls.append("offline")
                raise runtime.RuntimeDisconnected("offline")
            return await super().inspect()
    monkeypatch.setattr(runtime, "RuntimeChannel", Offline)
    cancel = SimpleNamespace(is_set=lambda: cancelled and "offline" in calls)
    with pytest.raises(BookingError) as error:
        runtime.refresh_entry(timeout_seconds=6, cancelled=cancel)
    assert error.value.code == ("CANCELLED" if cancelled else "LOGIN_CONFIRM_REQUIRED")
    assert calls.count("offline") <= 6 and "server_check" not in calls
    assert calls[-2:] == ["close_window", "detach"]


def test_closed_socket_send_invalidates_channel_without_replaying_command():
    from websockets.exceptions import ConnectionClosedOK
    from websockets.frames import Close
    class ClosedSocket:
        sends = 0
        async def send(self, _):
            self.sends += 1
            raise ConnectionClosedOK(Close(1000, ""), Close(1000, ""), True)
    async def run():
        channel = runtime.RuntimeChannel()
        socket = ClosedSocket()
        channel.socket = socket
        channel.ready.set()
        channel.contexts.add(12)
        with pytest.raises(runtime.RuntimeDisconnected):
            await channel.call("Input.dispatchMouseEvent")
        assert socket.sends == 1 and channel.socket is None
        assert not channel.ready.is_set() and not channel.contexts and not channel.pending
    asyncio.run(run())


def test_socket_replacement_wakes_pending_calls_and_ignores_old_page_events(monkeypatch):
    monkeypatch.setattr(runtime, "result_message", lambda raw: raw)
    class Socket:
        def __init__(self):
            self.messages = asyncio.Queue()
            self.sent = asyncio.Event()
        def __aiter__(self): return self
        async def __anext__(self):
            message = await self.messages.get()
            if message is None: raise StopAsyncIteration
            return message
        async def send(self, _): self.sent.set()
    async def run():
        channel = runtime.RuntimeChannel()
        old, new = Socket(), Socket()
        old_reader = asyncio.create_task(channel.receive(old))
        await channel.ready.wait()
        channel.contexts.add(12)
        pending = asyncio.create_task(channel.call("Runtime.enable", timeout=1))
        await old.sent.wait()
        channel.ready.clear()
        new_reader = asyncio.create_task(channel.receive(new))
        await channel.ready.wait()
        with pytest.raises(runtime.RuntimeDisconnected):
            await asyncio.wait_for(pending, 1)
        assert not channel.contexts
        await old.messages.put({"method": "Runtime.executionContextCreated", "params": {"context": {"id": 99}}})
        await old.messages.put(None)
        await old_reader
        assert channel.socket is new and channel.ready.is_set() and not channel.contexts
        resumed = asyncio.create_task(channel.call("Runtime.enable", timeout=1))
        await new.sent.wait()
        await new.messages.put({"id": channel.sequence, "result": {"resumed": True}})
        assert await asyncio.wait_for(resumed, 1) == {"resumed": True}
        await new.messages.put(None)
        await new_reader
        assert not channel.pending and not channel.ready.is_set()
    asyncio.run(run())


def test_socket_close_wakes_inflight_command_before_its_timeout():
    class Socket:
        def __init__(self):
            self.sent, self.closed = asyncio.Event(), asyncio.Event()
        def __aiter__(self): return self
        async def __anext__(self):
            await self.closed.wait()
            raise StopAsyncIteration
        async def send(self, _): self.sent.set()
    async def run():
        channel, socket = runtime.RuntimeChannel(), Socket()
        reader = asyncio.create_task(channel.receive(socket))
        await channel.ready.wait()
        pending = asyncio.create_task(channel.call("Runtime.enable", timeout=30))
        await socket.sent.wait()
        socket.closed.set()
        with pytest.raises(runtime.RuntimeDisconnected):
            await asyncio.wait_for(pending, 1)
        await reader
        assert not channel.pending
    asyncio.run(run())


@pytest.mark.parametrize("refreshes_after_native_login", [True, False])
def test_invalid_native_home_entry_recovers_once_without_replaying_rejected_ticket(monkeypatch, refreshes_after_native_login):
    calls, progress = environment(monkeypatch)
    class StaleHome(runtime.RuntimeChannel):
        async def inspect(self):
            if "server_check" not in calls:
                return [{"context": 1, "root": True, "visible": True, "labels": [], "back": False},
                        {"context": 2, "visible": True, "labels": ["场馆预约"]}]
            return await super().inspect()
    monkeypatch.setattr(runtime, "RuntimeChannel", StaleHome)
    def entry():
        if "场馆预约" not in calls: return ticket()
        if "授权登录" in calls and refreshes_after_native_login: return ticket("fresh")
        return ticket("rejected")
    monkeypatch.setattr(runtime, "discover_entry", entry)
    def auth(_):
        calls.append("server_check")
        return {"ok": calls.count("server_check") > 1, "code": "SESSION_EXPIRED",
                "account_fingerprint": runtime.account_digest(ticket())}
    monkeypatch.setattr(runtime, "check_auth", auth)
    if refreshes_after_native_login:
        result = runtime.refresh_entry(progress=progress.append)
        assert result["ok"] and result["native_login_performed"]
        assert calls.count("server_check") == 2
    else:
        with pytest.raises(BookingError) as error: runtime.refresh_entry(timeout_seconds=25, progress=progress.append)
        assert error.value.code == "SESSION_EXPIRED"
        assert calls.count("server_check") == 1
    assert calls.count("授权登录") == 1 and calls.count("reenter") == 2
    assert "login_entry_rejected" in [event["phase"] for event in progress]


def test_malformed_and_oversized_wire_messages_are_rejected():
    for raw in [b"\x00", b"\x22\x08abc", b"\x08\x80", b"x" * (wire.MAX_MESSAGE + 1)]:
        with pytest.raises(ValueError): wire.decode(raw)


def test_empty_or_non_cdp_messages_are_ignored():
    assert wire.result_message(wire.binary(3, "addJsContext")) is None
    assert wire.result_message(wire.binary(3, "chromeDevtoolsResult") + wire.binary(4, b"")) is None


def test_compressed_remote_result_decodes_without_exposing_payload():
    cdp = wire.binary(2, json.dumps({"id": 12, "result": {}}))
    envelope = wire.binary(3, "chromeDevtoolsResult") + wire.binary(4, zlib.compress(cdp)) + wire.integer(5, 1)
    assert wire.result_message(envelope) == {"id": 12, "result": {}}


def test_wire_request_matches_upstream_protobuf_encoder():
    # Generated once with upstream WARemoteDebug_DebugMessage / ChromeDevtools.
    expected = "08011a0e6368726f6d65446576746f6f6c7322340801122e7b226964223a312c226d6574686f64223a2252756e74696d652e656e61626c65222c22706172616d73223a7b7d7d1a0028003000"
    assert wire.command_envelope(1, {"id": 1, "method": "Runtime.enable", "params": {}}).hex() == expected
