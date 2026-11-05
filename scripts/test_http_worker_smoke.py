"""Exercise the shipped worker locally: model warmup, OCR parity and read-only guard."""
import base64
import ctypes
from ctypes import wintypes
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src-tauri/sidecars/http-booking"))
os.environ["OPENBLAS_NUM_THREADS"] = "1"
os.environ["OMP_NUM_THREADS"] = "1"
from PIL import Image
from xmu_http.client import CaptchaSolver
from xmu_http.session import BookingError

bundle = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / "src-tauri/resources/http-booking"
child = subprocess.Popen([str(bundle / "xmu-http-booking.exe"), "--serve-readonly"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
    text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW)
messages = queue.Queue()
def read_output():
    for line in child.stdout:
        if line.startswith("@@XMU_HTTP@@"):
            messages.put(json.loads(line[len("@@XMU_HTTP@@"):]))
    messages.put(None)
threading.Thread(target=read_output, daemon=True).start()

def rpc(payload):
    child.stdin.write(json.dumps(payload) + "\n")
    child.stdin.flush()
    deadline = time.monotonic() + 60
    while True:
        reply = messages.get(timeout=max(.01, deadline - time.monotonic()))
        assert reply is not None, "worker exited"
        if reply["kind"] == "result":
            return reply

def memory():
    class Counters(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("faults", wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ("peak_working", "working", "peak_paged", "paged",
                "peak_nonpaged", "nonpaged", "pagefile", "peak_pagefile", "private")]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Counters), wintypes.DWORD]
    handle = kernel.OpenProcess(0x1010, False, child.pid)
    assert handle
    try:
        counters = Counters()
        counters.cb = ctypes.sizeof(counters)
        assert psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb)
        return {"working_set_mib": counters.working / 2**20, "private_mib": counters.private / 2**20}
    finally:
        kernel.CloseHandle(handle)

try:
    binding = {"task":{"sport":"local","venue":"local","target_date":"2099-01-02",
        "target_time_start":"10:00","target_time_end":"11:00"}, "expected_account":"0" * 64}
    start = time.monotonic()
    warmed = rpc({"action":"warm", **binding})
    warm_seconds = time.monotonic() - start
    assert warmed["ok"] and not warmed["network_used"] and not warmed["booking_submitted"]
    warmed_memory = memory()
    assert rpc({"action":"ping", **binding})["model_load_count"] == 1
    solver = CaptchaSolver()
    cases = []
    for path in sorted((ROOT / "scripts/fixtures/captcha-ocr").glob("*.png")):
        cases.append((path.name, path.read_bytes()))
    assert cases, "OCR fixtures required"
    # Also exercise JPEG decoding without changing the original test files.
    with Image.open(io.BytesIO(cases[0][1])) as image:
        encoded = io.BytesIO()
        image.convert("RGB").save(encoded, format="JPEG")
        cases.append(("jpeg-reencode", encoded.getvalue()))
    predictions = []
    for name, content in cases:
        try:
            expected = {"prediction": solver.solve(content)}
        except BookingError as error:
            expected = {"code": error.code}
        result = rpc({"action":"self_test", "captcha_probe_base64":base64.b64encode(content).decode()})
        actual = {"prediction":result["captcha_probe"]} if result.get("ok") else {"code":result["code"]}
        assert actual == expected, (name, actual, expected)
        predictions.append({"case":name, **actual})
    assert rpc({"action":"book"})["code"] == "READONLY_WORKER"
    report = {"cases":len(cases), "ocr_source_parity":True, "readonly_guard":True,
        "network_used":False, "warm_seconds":warm_seconds, "warm_memory":warmed_memory, "predictions":predictions}
    out = ROOT / "artifacts/lite-build/worker-smoke.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))
finally:
    child.stdin.close()
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait()
