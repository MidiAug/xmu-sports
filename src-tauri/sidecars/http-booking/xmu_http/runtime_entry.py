"""Use the real mini-program's DOM controls, then verify a fresh H5 ticket.

No image matching, OS mouse/keyboard, CAPTCHA, or booking request is used here.
"""
from __future__ import annotations

import asyncio
import json
import os
import platform
import sys
import time
import traceback
from pathlib import Path

from .app import check_auth
from .recovery import account_digest, entry_digest, launch_miniprogram
from .runtime_wire import command_envelope, result_message
from .runtime_probe import probe_runtime
from .session import BookingError, discover_entry
from .windows_host import activate_xmu, close_xmu, xmu_windows

APP_ID = "wx81a2b2fa90759cb7"
FLUE_SHA256 = "60630cc1ba346b0e485fbf1981a07a650d1566e004c5b2b84933544860e2b3cf"
HOOK_CONFIG = {"Version": 25560, "LoadStartHookOffset": "0x2ce7cf0",
               "CDPFilterHookOffset": "0x39789d0",
               "SceneOffsets": [64, 1536, 8, 1472, 16, 456]}

VISIBLE = """let w=window,visible=true;while(w.frameElement){const e=w.frameElement,r=e.getBoundingClientRect(),s=w.parent.getComputedStyle(e);if(r.width===0||r.height===0||r.right<0||r.bottom<0||r.x>w.parent.innerWidth||r.y>w.parent.innerHeight||s.visibility==='hidden'||s.display==='none')visible=false;w=w.parent;}"""
INSPECT = """const text=document.body?.innerText||'';return {visible,labels:['立即登录','场馆预约','授权登录','身份认证','学号'].filter(s=>text.includes(s)),agreement:document.querySelector('wx-radio.auth__radio-icon')?.getAttribute('aria-checked'),root:!!document.querySelector('.nav_bar_wrapper'),back:!![...document.querySelectorAll('.nav_bar .back')].find(e=>e.getBoundingClientRect().width>0)}"""


def runtime_process():
    import frida
    try:
        device = frida.get_local_device()
        candidates = [p for p in device.enumerate_processes(scope="full")
                      if p.name.lower() == "wechatappex.exe"
                      and not any(arg.startswith("--type=") for arg in p.parameters.get("argv", []))]
    except Exception as error:
        raise BookingError("RUNTIME_PROBE_FAILED", "无法查询微信小程序进程，请重启微信后重试，并导出诊断日志",
                           details={"exception_type": type(error).__name__, "probe_stage": "enumerate_processes"}) from error
    if not candidates:
        return device, None
    if len(candidates) != 1:
        raise BookingError("RUNTIME_BUSY", "检测到多个微信小程序内核，请确认仅在目标微信账号运行后重试",
                           details={"runtime_process_count": len(candidates)})
    return device, candidates[0]


class RuntimeDisconnected(ConnectionError):
    """The page connection changed; inspect again before issuing more input."""


class RuntimeChannel:
    def __init__(self):
        self.socket = None
        self.ready = asyncio.Event()
        self.contexts = set()
        self.pending = {}
        self.sequence = 1000
        self.session = self.script = self.server = None
        self.host_missing = False
        self.restored = False
        self.diagnostics = {"worker_build": "0.1.10-reconnect-20261018",
                            "os": platform.system(), "os_release": platform.release(),
                            "os_version": platform.version(),
                            "architecture": platform.machine(), "frozen": bool(getattr(sys, "frozen", False)),
                            "supported_runtime": HOOK_CONFIG["Version"], "runtime_found": False,
                            "runtime_folder": "unknown", "runtime_sha256": None,
                            "runtime_probe_stage": "not_started"}

    def disconnect(self, socket):
        if self.socket is socket:
            self.socket = None
            self.ready.clear()
            self.contexts.clear()
            self.diagnostics["bridge_disconnections"] = self.diagnostics.get("bridge_disconnections", 0) + 1
        for owner, future in list(self.pending.values()):
            if owner is socket and not future.done():
                future.set_exception(RuntimeDisconnected("runtime connection changed"))

    async def receive(self, socket):
        from websockets.exceptions import ConnectionClosed
        if self.socket is not None:
            self.disconnect(self.socket)
        self.socket = socket
        self.contexts.clear()
        self.diagnostics["bridge_connections"] = self.diagnostics.get("bridge_connections", 0) + 1
        self.ready.set()
        try:
            async for raw in socket:
                if self.socket is not socket:
                    continue
                try:
                    message = result_message(raw)
                except (ValueError, TypeError, UnicodeDecodeError):
                    continue
                if not message:
                    continue
                if message.get("method") == "Runtime.executionContextCreated":
                    self.contexts.add(message["params"]["context"]["id"])
                elif message.get("method") == "Runtime.executionContextDestroyed":
                    self.contexts.discard(message["params"]["executionContextId"])
                pending = self.pending.get(message.get("id"))
                if pending and pending[0] is socket and not pending[1].done():
                    pending[1].set_result(message)
        except ConnectionClosed:
            # Re-entering or closing the mini-program reconnects this channel.
            pass
        finally:
            self.disconnect(socket)

    async def start(self):
        self.diagnostics["runtime_probe_stage"] = "enumerate_processes"
        device, process = runtime_process()
        self.host_missing = process is None
        self.diagnostics["runtime_found"] = process is not None
        if process is None:
            raise BookingError("RUNTIME_UNAVAILABLE", "微信小程序内核尚未启动")
        self.diagnostics["runtime_probe_stage"] = "locate_and_hash"
        probe_runtime(process, self.diagnostics, FLUE_SHA256)
        self.diagnostics["runtime_probe_stage"] = "bridge_listen"
        from websockets.asyncio.server import serve
        try:
            self.server = await serve(self.receive, "127.0.0.1", 9421, max_size=4 * 1024 * 1024, close_timeout=1)
        except OSError as error:
            raise BookingError("RUNTIME_BUSY", "小程序调试通道正在被其他进程使用，请关闭调试工具后重试") from error
        source = (Path(__file__).parent / "assets" / "auth-hook.js").read_text(encoding="utf-8")
        self.diagnostics["runtime_probe_stage"] = "attach"
        try:
            self.session = device.attach(process.pid)
        except Exception as error:
            raise BookingError("RUNTIME_ATTACH_FAILED", "已识别支持的内核，但无法连接微信小程序进程，请确认微信和本软件以同一用户运行后重试",
                               details={"exception_type": type(error).__name__}) from error
        self.diagnostics["runtime_probe_stage"] = "load_script"
        self.script = self.session.create_script(source.replace("@@CONFIG@@", json.dumps(HOOK_CONFIG)))
        self.script.load()
        self.diagnostics["bridge_attached"] = True
        self.diagnostics["runtime_probe_stage"] = "ready"

    async def call(self, method, params=None, timeout=8):
        from websockets.exceptions import ConnectionClosed
        self.diagnostics["runtime_command"] = method
        await asyncio.wait_for(self.ready.wait(), timeout)
        socket = self.socket
        if socket is None:
            raise RuntimeDisconnected("runtime connection changed")
        self.sequence += 1
        sequence = self.sequence
        future = asyncio.get_running_loop().create_future()
        self.pending[sequence] = (socket, future)
        try:
            await socket.send(command_envelope(sequence, {"id": sequence, "method": method, "params": params or {}}))
            message = await asyncio.wait_for(future, timeout)
            if self.socket is not socket:
                raise RuntimeDisconnected("runtime reply belongs to an old connection")
            if message.get("error"):
                raise ValueError("runtime command rejected")
            return message.get("result", {})
        except ConnectionClosed as error:
            self.disconnect(socket)
            raise RuntimeDisconnected("runtime connection closed during command") from error
        finally:
            self.pending.pop(sequence, None)
            if not future.done():
                future.cancel()
            elif not future.cancelled():
                future.exception()

    async def evaluate(self, context, code):
        expression = ("JSON.stringify((()=>{if(!location.pathname.startsWith('/" + APP_ID
                      + "/'))return null;" + VISIBLE + code + "})())")
        result = await self.call("Runtime.evaluate", {"expression": expression, "returnByValue": True, "contextId": context})
        if result.get("exceptionDetails"):
            return None
        value = result.get("result", {}).get("value")
        return json.loads(value) if value else None

    async def inspect(self):
        await self.call("Runtime.disable")
        self.contexts.clear()
        await self.call("Runtime.enable")
        states = []
        for context in sorted(self.contexts):
            try:
                state = await self.evaluate(context, INSPECT)
                if state and state["visible"]:
                    states.append({"context": context, **state})
            except RuntimeDisconnected:
                raise
            except (ValueError, asyncio.TimeoutError):
                continue
        return states

    async def click(self, context, label):
        if label not in {"立即登录", "场馆预约", "授权登录", "agreement", "back"}:
            raise ValueError("unexpected control")
        if label == "agreement":
            selector = "[...document.querySelectorAll('wx-radio.auth__radio-icon')]"
        elif label == "back":
            selector = "[...document.querySelectorAll('.nav_bar .back')]"
        else:
            selector = "[...document.querySelectorAll('*')].filter(e=>e.textContent.trim()===" + json.dumps(label) + ")"
        code = ("if(!visible)return null;const nodes=" + selector + ";nodes.sort((a,b)=>a.querySelectorAll('*').length-b.querySelectorAll('*').length);"
                "const e=nodes.find(e=>e.getBoundingClientRect().width>0);if(!e||e.getAttribute('aria-disabled')==='true')return null;"
                + ("if(e.getAttribute('aria-checked')==='true')return {already:true};" if label == "agreement" else "")
                + "const r=e.getBoundingClientRect();let x=r.x+r.width/2,y=r.y+r.height/2,p=window;while(p.frameElement){const f=p.frameElement.getBoundingClientRect();x+=f.x;y+=f.y;p=p.parent;}return{x,y};")
        point = await self.evaluate(context, code)
        if not point:
            return False
        if point.get("already"):
            return True
        activate_xmu()
        for kind in ("mouseMoved", "mousePressed", "mouseReleased"):
            params = {"type": kind, "x": point["x"], "y": point["y"]}
            if kind != "mouseMoved":
                params.update(button="left", clickCount=1)
            await self.call("Input.dispatchMouseEvent", params)
        return True

    async def reenter(self, root):
        return await self.evaluate(root, "if(!visible)return null;const e=[...document.querySelectorAll('.menu_item')].find(e=>e.textContent.trim()==='重新进入小程序');if(!e)return null;e.click();return {reentered:true};")

    async def close(self):
        # Restore temporary native fields before closing the remote channel.
        if self.script:
            await self.restore()
            try:
                self.script.unload()
            except Exception:
                pass
        if self.session:
            try:
                self.session.detach()
            except Exception:
                pass
        if self.server:
            self.server.close()
            await self.server.wait_closed()

    async def restore(self):
        if self.script and not self.restored:
            # Never re-read captured pointers after WM_CLOSE frees the app.
            self.restored = True
            try:
                self.script.exports_sync.restore()
            except Exception:
                pass
            await asyncio.sleep(0.5)


async def _refresh(expected_account=None, cancelled=None, progress=None, timeout_seconds=90):
    if os.name != "nt":
        raise BookingError("RUNTIME_UNSUPPORTED", "自动入口当前仅支持 Windows 电脑微信")
    last_emitted = (None, 0)
    def emit(phase, **fields):
        nonlocal last_emitted
        now = time.monotonic()
        if phase in {"waiting_native_page", "waiting_h5_entry", "native_login_pending", "waiting_miniprogram_focus", "runtime_reconnecting"} and last_emitted[0] == phase and now - last_emitted[1] < 3:
            return
        last_emitted = (phase, now)
        if progress:
            progress({"phase": phase, **fields})
    def check_cancel():
        if cancelled is not None and cancelled.is_set():
            raise BookingError("CANCELLED", "登录准备已取消")
    initial = None
    try:
        initial = discover_entry()
    except BookingError as error:
        if error.code not in {"NO_ENTRY", "INVALID_ENTRY"}:
            raise
    if initial:
        initial_account = account_digest(initial)
        if expected_account and expected_account != initial_account:
            raise BookingError("ACCOUNT_CHANGED", "当前微信账号与任务账号不一致")
        expected_account = initial_account
    old = entry_digest(initial) if initial else None
    channel = RuntimeChannel()
    started = time.monotonic()
    venue_clicked = False
    reentered = False
    seen = {}
    completed = None
    opened = False
    last_action = None
    last_action_at = -float("inf")
    focus_blocked = False
    entry_rejection = None
    try:
        check_cancel()
        emit("runtime_connecting", diagnostics=getattr(channel, "diagnostics", {}),
             cached_entry_present=initial is not None)
        try:
            await channel.start()
        except BookingError as error:
            if error.code != "RUNTIME_UNAVAILABLE" or not getattr(channel, "host_missing", False):
                raise
            # Boot the registered app normally, then close its window before
            # attaching. Opening it again gives the hook a fresh load event.
            emit("launching_runtime")
            launch_miniprogram()
            opened = True
            boot_deadline = min(started + timeout_seconds, time.monotonic() + 12)
            while time.monotonic() < boot_deadline:
                check_cancel()
                _, process = runtime_process()
                if process is not None:
                    break
                await asyncio.sleep(0.25)
            else:
                raise BookingError("RUNTIME_UNAVAILABLE", "未能启动微信小程序内核，请确认电脑微信已登录后重试")
            close_xmu()
            await asyncio.sleep(0.5)
            check_cancel()
            await channel.start()
        emit("runtime_ready", diagnostics=getattr(channel, "diagnostics", {}))
        emit("launching_login")
        launch_miniprogram()
        opened = True
        activated = False
        while time.monotonic() - started < timeout_seconds:
            check_cancel()
            if not activated:
                activated = activate_xmu()
                focus_blocked = not activated
            if venue_clicked:
                try:
                    entry = discover_entry()
                    if expected_account and account_digest(entry) != expected_account:
                        raise BookingError("ACCOUNT_CHANGED", "重新登录的账号与原任务不一致")
                    if entry_digest(entry) != old:
                        emit("validating_login_entry")
                        result = await asyncio.to_thread(check_auth, entry.profile)
                        check_cancel()
                        if result.get("account_fingerprint") != account_digest(entry):
                            raise BookingError("ACCOUNT_CHANGED", "核验期间登录账号发生变化")
                        if result.get("ok"):
                            emit("fresh_entry_ready")
                            completed = {**result, "recovery_mode": "runtime_dom", "fresh_entry_observed": True,
                                         "automatic_native_login": True, "native_login_performed":
                                         any(label == "授权登录" for _, label in seen)}
                            return completed
                        recoverable = result.get("code") == "SESSION_EXPIRED" or (
                            result.get("code") == "API_REJECTED" and result.get("phase") == "authenticating")
                        if recoverable and entry_rejection is None and not any(label == "授权登录" for _, label in seen):
                            # A cached home page may still render after another
                            # device has invalidated its native token. Re-enter
                            # once and let the original app detect/recover it.
                            entry_rejection = result
                            emit("login_entry_rejected")
                            old = entry_digest(entry)
                            venue_clicked = reentered = activated = False
                            seen.clear()
                            last_action = None
                            last_action_at = -float("inf")
                            continue
                        raise BookingError(result.get("code", "SESSION_EXPIRED"), result.get("message", "新入口仍不可用"))
                except BookingError as error:
                    if error.code not in {"NO_ENTRY", "INVALID_ENTRY", "SESSION_READ_FAILED"}:
                        raise
                emit("waiting_h5_entry")
            else:
                try:
                    states = await channel.inspect()
                except RuntimeDisconnected:
                    emit("runtime_reconnecting", diagnostics=getattr(channel, "diagnostics", {}))
                    await asyncio.sleep(1)
                    continue
                except (asyncio.TimeoutError, ValueError):
                    await asyncio.sleep(1)
                    continue
                root = next((s for s in states if s.get("root")), None)
                if not reentered and root:
                    emit("runtime_reenter")
                    try:
                        await channel.reenter(root["context"])
                    except (RuntimeDisconnected, asyncio.TimeoutError, ValueError):
                        # Re-entry can close the socket before its reply arrives.
                        # Do not replay the command; wait for the new page instead.
                        emit("runtime_reconnecting", diagnostics=getattr(channel, "diagnostics", {}))
                    reentered = True
                    activated = False
                    await asyncio.sleep(2)
                    continue
                if not reentered:
                    await asyncio.sleep(1)
                    continue
                if not activated:
                    emit("waiting_miniprogram_focus", remaining_seconds=max(0, int(timeout_seconds - (time.monotonic() - started))))
                    await asyncio.sleep(1)
                    continue
                acted = False
                for label, phase in (("立即登录", "native_login_opening"), ("授权登录", "native_authorizing"), ("场馆预约", "entering_stadium")):
                    page = next((s for s in states if not s.get("root") and label in s["labels"]), None)
                    if not page:
                        continue
                    key = (page["context"], label)
                    attempts = sum(count for (_, attempted), count in seen.items() if attempted == label)
                    if attempts >= 2 or (last_action and last_action[1] == label and time.monotonic() - last_action_at < 5):
                        continue
                    if label == "授权登录" and page.get("agreement") != "true":
                        emit("native_agreement")
                        try:
                            await channel.click(page["context"], "agreement")
                        except (RuntimeDisconnected, asyncio.TimeoutError, ValueError):
                            emit("runtime_reconnecting", diagnostics=getattr(channel, "diagnostics", {}))
                            acted = True
                            break
                        await asyncio.sleep(0.3)
                    emit(phase)
                    try:
                        clicked = await channel.click(page["context"], label)
                    except (RuntimeDisconnected, asyncio.TimeoutError, ValueError):
                        # Input may already have reached the page. Preserve the
                        # cooldown and inspect its actual state before trying again.
                        seen[key] = seen.get(key, 0) + 1
                        last_action, last_action_at = key, time.monotonic()
                        activated = False
                        emit("runtime_reconnecting", diagnostics=getattr(channel, "diagnostics", {}))
                        acted = True
                        break
                    if clicked:
                        seen[key] = seen.get(key, 0) + 1
                        last_action = key
                        last_action_at = time.monotonic()
                        activated = False
                        acted = True
                        if label == "场馆预约":
                            venue_clicked = True
                            emit("waiting_h5_entry")
                        break
                if not acted and time.monotonic() - last_action_at < 10:
                    emit("native_login_pending" if last_action and last_action[1] in {"立即登录", "授权登录"} else "waiting_native_page")
                elif not acted and root and root.get("back"):
                    emit("returning_home")
                    try:
                        await channel.click(root["context"], "back")
                    except (RuntimeDisconnected, asyncio.TimeoutError, ValueError):
                        emit("runtime_reconnecting", diagnostics=getattr(channel, "diagnostics", {}))
                elif not acted:
                    emit("waiting_native_page")
            await asyncio.sleep(1)
        if focus_blocked:
            raise BookingError("LOGIN_FOCUS_REQUIRED", "体育小程序未获得操作焦点，请暂时退出全屏或切换到小程序后重试")
        if entry_rejection is not None:
            raise BookingError(entry_rejection.get("code", "SESSION_EXPIRED"), "场馆登录已失效，自动重新登录未完成，请打开体育小程序手动确认")
        raise BookingError("LOGIN_CONFIRM_REQUIRED", "自动进入场馆未完成，请在电脑微信确认体育登录与身份认证后重试")
    except BookingError as error:
        error.details = {**(error.details or {}), "failure_phase": last_emitted[0],
                         "compatibility": dict(getattr(channel, "diagnostics", {}))}
        raise
    except Exception as error:
        raise BookingError("RUNTIME_BRIDGE_FAILED", "小程序自动入口异常，请导出诊断日志，或手动登录后进入场馆预约",
                           details={"exception_type": type(error).__name__, "failure_phase": last_emitted[0],
                                    "frames": [{"file": Path(frame.filename).name, "function": frame.name, "line": frame.lineno}
                                               for frame in traceback.extract_tb(error.__traceback__)[-6:]],
                                    "compatibility": dict(getattr(channel, "diagnostics", {}))}) from error
    finally:
        emit("closing_miniprogram", diagnostics=getattr(channel, "diagnostics", {}),
             opened=opened, venue_entered=venue_clicked,
             native_login_performed=any(key[1] == "授权登录" for key in seen),
             recovery_elapsed_ms=int((time.monotonic() - started) * 1000))
        try:
            await channel.restore()
            if opened:
                close_xmu()
                await asyncio.sleep(0.3)
            if completed is not None:
                completed["mini_program_closed"] = not any(w["visible"] for w in xmu_windows())
                if not completed["mini_program_closed"]:
                    raise BookingError("RUNTIME_CLOSE_FAILED", "登录票据已验证，但小程序尚未关闭，请手动关闭后重试")
        finally:
            await channel.close()


def refresh_entry(**kwargs):
    try:
        return asyncio.run(_refresh(**kwargs))
    except BookingError:
        raise
    except ImportError as error:
        raise BookingError("HTTP_COMPONENT_MISSING", "自动入口组件未完整安装，请安装新版软件",
                           details={"exception_type": type(error).__name__}) from error
    except Exception as error:
        raise BookingError("RUNTIME_BRIDGE_FAILED", "小程序自动入口异常，请手动确认登录并进入场馆预约后重试",
                           details={"exception_type": type(error).__name__}) from error
