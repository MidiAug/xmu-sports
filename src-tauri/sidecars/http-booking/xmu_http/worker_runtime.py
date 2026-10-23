"""One scheduled run owns one preloaded solver. No cookies or captcha cached."""
import hashlib
import json
import os
import re

from .client import CaptchaSolver, Target
from .session import BookingError


class WorkerRuntime:
    def __init__(self):
        self.solver = None
        self.binding = None
        self.model_load_count = 0
        self.book_consumed = False

    @staticmethod
    def context(payload):
        account = payload.get("expected_account")
        if not isinstance(account, str) or not re.fullmatch(r"[0-9a-f]{64}", account):
            raise BookingError("HTTP_AUTH_UNVERIFIED", "预热需要已核验的原账号")
        task = payload.get("task")
        if not isinstance(task, dict):
            raise BookingError("INVALID_TARGET", "预热缺少冻结任务")
        Target(sport=task["sport"],venue=task["venue"],date=task["target_date"],
               time=task["target_time_start"],end=task["target_time_end"],court=task.get("prefer_court") or None)
        # Include fallback policy/revision too: editing a task cannot reuse a
        # process bound to a different intended operation.
        return account, hashlib.sha256(json.dumps(task,sort_keys=True,ensure_ascii=False).encode()).hexdigest()

    def warm(self, payload, cancelled):
        binding = self.context(payload)
        if self.binding is not None and self.binding != binding:
            raise BookingError("WARM_CONTEXT_CHANGED", "预热账号或任务已改变")
        if self.book_consumed:
            raise BookingError("WORKER_ALREADY_CONSUMED", "该进程已处理过预约，不允许重放")
        if cancelled.is_set():
            raise BookingError("CANCELLED", "预热已取消")
        if self.solver is None:
            solver = CaptchaSolver()
            solver.preload()
            self.solver = solver
            self.model_load_count += 1
        self.binding = binding
        if cancelled.is_set():
            raise BookingError("CANCELLED", "预热已取消")
        return self.ready()

    def ready(self):
        return {"ok":True,"state":"worker_warmed","model_ready":self.solver is not None,"inference_preloaded":self.solver is not None,
                "model_load_count":self.model_load_count,"process_id":os.getpid(),
                "network_used":False,"booking_submitted":False}

    def check(self, payload):
        if self.binding is None or self.binding != self.context(payload) or self.solver is None:
            raise BookingError("WARM_CONTEXT_CHANGED", "预热进程未就绪或账号任务不匹配")
        if self.book_consumed:
            raise BookingError("WORKER_ALREADY_CONSUMED", "该进程已处理过预约，不允许重放")
        return self.ready()

    def consume(self, payload):
        self.check(payload)
        # Mark before entering booking, even if validation/cancellation fails.
        self.book_consumed = True
        return self.solver


def watch_parent():
    """A real process handle avoids PID reuse; also stop on stdin EOF normally."""
    raw = os.environ.get("XMU_HTTP_PARENT_PID")
    if os.name != "nt" or not raw or not raw.isdigit():
        return
    import ctypes
    import threading
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE,wintypes.DWORD]
    kernel.WaitForSingleObject.restype = wintypes.DWORD
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x00100000,False,int(raw))  # SYNCHRONIZE only
    if not handle:
        raise BookingError("WORKER_PARENT_UNAVAILABLE", "无法绑定预热进程的宿主生命周期")
    def watch():
        try:
            status=kernel.WaitForSingleObject(handle,0xFFFFFFFF)
            os._exit(0 if status==0 else 1)
        finally:
            kernel.CloseHandle(handle)
    threading.Thread(target=watch,daemon=True).start()
