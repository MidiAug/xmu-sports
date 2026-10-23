"""Window ownership checks and WM_CLOSE; no screenshots or desktop input."""
import os

from .session import BookingError


def xmu_windows():
    if os.name != "nt":
        raise BookingError("LAUNCH_MP_NEED_DESKTOP", "当前自动入口仅支持 Windows 电脑微信")
    import ctypes
    from ctypes import wintypes as w
    user = ctypes.WinDLL("user32", use_last_error=True)
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
    kernel.CloseHandle.argtypes = [w.HANDLE]
    user.GetWindowThreadProcessId.argtypes = [w.HWND, ctypes.POINTER(w.DWORD)]
    user.GetWindowTextW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
    user.IsWindowVisible.argtypes = [w.HWND]
    callback = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    windows = []
    @callback
    def visit(hwnd, _):
        title = ctypes.create_unicode_buffer(512)
        user.GetWindowTextW(hwnd, title, 512)
        if not any(word in title.value for word in ("厦大体育", "厦门大学")):
            return True
        pid = w.DWORD()
        user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        handle = kernel.OpenProcess(0x1000, False, pid.value)
        if not handle:
            return True
        path = ctypes.create_unicode_buffer(4096)
        size = w.DWORD(4096)
        try:
            if (kernel.QueryFullProcessImageNameW(handle, 0, path, ctypes.byref(size))
                    and path.value.lower().endswith("wechatappex.exe")):
                windows.append({"hwnd": hwnd, "pid": pid.value, "visible": bool(user.IsWindowVisible(hwnd))})
        finally:
            kernel.CloseHandle(handle)
        return True
    user.EnumWindows(visit, 0)
    return windows


def close_xmu():
    import ctypes
    from ctypes import wintypes as w
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
    for window in xmu_windows():
        if window["visible"]:
            user.PostMessageW(window["hwnd"], 0x10, 0, 0)


def activate_xmu():
    """Activate only the verified sports window before runtime input events."""
    import ctypes
    from ctypes import wintypes as w
    windows = [item for item in xmu_windows() if item["visible"]]
    if len(windows) != 1:
        return False
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.SetForegroundWindow.argtypes = [w.HWND]
    user.SetForegroundWindow.restype = w.BOOL
    user.ShowWindow.argtypes = [w.HWND, ctypes.c_int]
    user.IsIconic.argtypes = [w.HWND]
    hwnd = windows[0]["hwnd"]
    if user.IsIconic(hwnd):
        user.ShowWindow(hwnd, 9)
    return bool(user.SetForegroundWindow(hwnd))
