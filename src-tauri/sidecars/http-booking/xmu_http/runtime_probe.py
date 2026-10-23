"""Locate the running WMPF DLL without attaching or exporting local paths.

Loaded modules take precedence over files from other installed runtime versions.
All paths stay inside this module; only allowlisted metadata goes into diagnostics.
"""
from __future__ import annotations

import ctypes
import hashlib
import os
import re
import struct
from itertools import islice
from pathlib import Path

from .session import BookingError


def native_process_paths(pid):
    """Read executable/module paths using Win32, independently of Frida argv."""
    diagnostics = {"image_query": "unavailable", "module_query": "unavailable"}
    if os.name != "nt":
        return None, [], diagnostics
    from ctypes import wintypes as w
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.CloseHandle.restype = w.BOOL
    kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, ctypes.POINTER(w.DWORD)]
    kernel.QueryFullProcessImageNameW.restype = w.BOOL
    kernel.K32EnumProcessModulesEx.argtypes = [w.HANDLE, ctypes.POINTER(w.HMODULE), w.DWORD,
                                             ctypes.POINTER(w.DWORD), w.DWORD]
    kernel.K32EnumProcessModulesEx.restype = w.BOOL
    kernel.K32GetModuleFileNameExW.argtypes = [w.HANDLE, w.HMODULE, w.LPWSTR, w.DWORD]
    kernel.K32GetModuleFileNameExW.restype = w.DWORD
    executable, modules = None, []
    handle = kernel.OpenProcess(0x1000, False, pid)  # QUERY_LIMITED_INFORMATION
    if handle:
        try:
            buffer = ctypes.create_unicode_buffer(32768)
            size = w.DWORD(len(buffer))
            if kernel.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
                executable = Path(buffer.value)
                diagnostics["image_query"] = "ok"
            else:
                diagnostics.update(image_query="failed", image_query_error=ctypes.get_last_error())
        finally:
            kernel.CloseHandle(handle)
    else:
        diagnostics.update(image_query="failed", image_query_error=ctypes.get_last_error())
    handle = kernel.OpenProcess(0x410, False, pid)  # QUERY_INFORMATION | VM_READ
    if not handle:
        diagnostics.update(module_query="failed", module_query_error=ctypes.get_last_error())
        return executable, modules, diagnostics
    try:
        handles = (w.HMODULE * 4096)()
        needed = w.DWORD()
        if not kernel.K32EnumProcessModulesEx(handle, handles, ctypes.sizeof(handles), ctypes.byref(needed), 3):
            diagnostics.update(module_query="failed", module_query_error=ctypes.get_last_error())
            return executable, modules, diagnostics
        if needed.value > ctypes.sizeof(handles):
            diagnostics["module_query"] = "truncated"
            return executable, modules, diagnostics
        count = needed.value // ctypes.sizeof(w.HMODULE)
        diagnostics.update(module_query="ok", module_count=count)
        for module in handles[:count]:
            buffer = ctypes.create_unicode_buffer(32768)
            length = kernel.K32GetModuleFileNameExW(handle, module, buffer, len(buffer))
            if not length or length >= len(buffer) - 1:
                diagnostics.update(module_query="partial", module_path_error=ctypes.get_last_error())
                continue
            path = Path(buffer.value)
            if path.name.lower() == "flue.dll":
                modules.append(path)
    finally:
        kernel.CloseHandle(handle)
    return executable, modules, diagnostics


def file_version(path):
    """Return only the numeric fixed file version (never arbitrary PE strings)."""
    if os.name != "nt":
        return None
    from ctypes import wintypes as w
    version = ctypes.WinDLL("version", use_last_error=True)
    version.GetFileVersionInfoSizeW.argtypes = [w.LPCWSTR, ctypes.POINTER(w.DWORD)]
    version.GetFileVersionInfoSizeW.restype = w.DWORD
    version.GetFileVersionInfoW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, w.LPVOID]
    version.GetFileVersionInfoW.restype = w.BOOL
    version.VerQueryValueW.argtypes = [w.LPCVOID, w.LPCWSTR, ctypes.POINTER(w.LPVOID), ctypes.POINTER(w.UINT)]
    version.VerQueryValueW.restype = w.BOOL
    size = version.GetFileVersionInfoSizeW(str(path), None)
    if not size or size > 1024 * 1024:
        return None
    buffer = ctypes.create_string_buffer(size)
    if not version.GetFileVersionInfoW(str(path), 0, size, buffer):
        return None
    pointer, length = w.LPVOID(), w.UINT()
    if not version.VerQueryValueW(buffer, "\\", ctypes.byref(pointer), ctypes.byref(length)) or length.value < 52:
        return None
    fixed = ctypes.cast(pointer, ctypes.POINTER(w.DWORD))
    if fixed[0] != 0xFEEF04BD:
        return None
    return ".".join(str(part) for part in (fixed[2] >> 16, fixed[2] & 65535,
                                           fixed[3] >> 16, fixed[3] & 65535))


def numeric_folder(path):
    for parent in list(path.parents)[:5]:
        if re.fullmatch(r"[0-9]+(?:\.[0-9]+)*", parent.name) and len(parent.name) <= 32:
            return parent.name
        if parent.name.lower() not in {"x64", "x86", "arm64", "runtime", "bin"}:
            break
    return "unknown"


def pe_architecture(path):
    try:
        with path.open("rb") as stream:
            header = stream.read(64)
            if len(header) != 64 or header[:2] != b"MZ":
                return "unknown"
            offset = struct.unpack_from("<I", header, 60)[0]
            if offset > 1024 * 1024:
                return "unknown"
            stream.seek(offset)
            pe = stream.read(6)
            if len(pe) != 6 or pe[:4] != b"PE\0\0":
                return "unknown"
            return {0x8664: "x64", 0x14c: "x86", 0xaa64: "arm64"}.get(struct.unpack_from("<H", pe, 4)[0], "unknown")
    except OSError:
        return "unknown"


def _path(value):
    if not isinstance(value, (str, os.PathLike)) or not value:
        return None
    # Some process providers preserve quotes around argv[0]. Do not split on spaces.
    path = Path(os.fspath(value).strip('"'))
    return path if path.is_absolute() else None


def _disk_candidates(executable, diagnostics):
    sibling = executable.with_name("flue.dll")
    if sibling.is_file():
        return [(sibling, "exe_sibling")]
    base = executable.parent
    roots = dict.fromkeys((base, base.parent, base / "RadiumWMPF", base.parent / "RadiumWMPF",
                          base / "WMPF", base.parent / "WMPF"))
    candidates = {}
    errors = []
    for root in roots:
        # Never walk a drive root or recursively search user data / other installs.
        if root == root.parent:
            continue
        try:
            children = list(islice(root.iterdir(), 129))
        except OSError as error:
            if not isinstance(error, FileNotFoundError):
                errors.append(getattr(error, "winerror", None) or error.errno)
            continue
        if len(children) > 128:
            diagnostics["layout_search_truncated"] = True
            continue
        for child in children:
            if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)*", child.name) or len(child.name) > 32 or not child.is_dir():
                continue
            for suffix in ("", "x64", "x86", "arm64", "runtime", "bin", "bin/x64", "bin/x86", "bin/arm64"):
                path = child / suffix / "flue.dll"
                if path.is_file():
                    candidates[path.resolve()] = (path, "version_directory")
    if errors:
        diagnostics["layout_search_errors"] = sorted(set(errors))[:8]
    return list(candidates.values())


def probe_runtime(process, diagnostics, supported_hash):
    """Locate/hash exactly one runtime. Never select a DLL just because it is supported."""
    executable, modules, native = native_process_paths(process.pid)
    diagnostics.update(native)
    parameters = process.parameters
    argv = parameters.get("argv") or []
    diagnostics.update(process_path_present=bool(parameters.get("path")), process_argv_present=bool(argv),
                       runtime_supported=False)
    source = "win32_image"
    if executable is None:
        executable, source = _path(parameters.get("path")), "frida_path"
    if executable is None:
        executable, source = _path(argv[0] if argv else None), "frida_argv"
    diagnostics["runtime_path_source"] = source if executable else "unavailable"
    if executable:
        diagnostics["runtime_exe_version"] = file_version(executable)
        diagnostics["runtime_exe_architecture"] = pe_architecture(executable)
    # Module paths identify the DLL that will actually receive the hook, even when
    # multiple old/updated versions are installed next to the executable.
    candidates = [(path, "loaded_module") for path in dict.fromkeys(modules)]
    if not candidates and executable:
        candidates = _disk_candidates(executable, diagnostics)
    diagnostics["runtime_candidate_count"] = len(candidates)
    diagnostics["runtime_candidates"] = [{"source": source, "folder": numeric_folder(path)}
                                         for path, source in candidates[:8]]

    def fail(code, message):
        diagnostics["runtime_probe_result"] = code
        raise BookingError(code, message, details={"compatibility": dict(diagnostics)})

    if not modules and diagnostics.get("layout_search_truncated"):
        fail("RUNTIME_AMBIGUOUS", "微信内核目录搜索未完成，请重启微信后重试；仍失败请导出诊断日志")
    if not candidates:
        if executable is None:
            fail("RUNTIME_PATH_UNAVAILABLE", "未能读取微信小程序内核位置，请重启电脑微信后重试；仍失败请导出诊断日志")
        if diagnostics.get("layout_search_errors"):
            fail("RUNTIME_FILE_UNREADABLE", "无法检查微信内核目录，请确认微信和本软件以同一用户运行，并导出诊断日志")
        fail("RUNTIME_FILE_MISSING", "已找到微信小程序进程，但未找到内核文件，请重启微信后重试；仍失败请导出诊断日志")
    if len(candidates) != 1:
        fail("RUNTIME_AMBIGUOUS", "发现多个或无法完整识别的微信内核目录，请重启微信后重试；仍失败请导出诊断日志")
    path, source = candidates[0]
    diagnostics.update(runtime_folder=numeric_folder(path), runtime_dll_source=source,
                       runtime_file_version=file_version(path), runtime_architecture=pe_architecture(path))
    try:
        # Streaming avoids loading a large runtime DLL into worker memory.
        digest = hashlib.sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        actual_hash = digest.hexdigest()
        diagnostics["runtime_file_size"] = path.stat().st_size
    except OSError as error:
        diagnostics["runtime_file_error"] = getattr(error, "winerror", None) or error.errno
        if isinstance(error, FileNotFoundError):
            fail("RUNTIME_FILE_MISSING", "微信内核文件已变化或被移除，请重启微信后重试")
        fail("RUNTIME_FILE_UNREADABLE", "无法读取微信内核文件，请确认微信和本软件以同一用户运行，并导出诊断日志")
    diagnostics.update(runtime_sha256=actual_hash, runtime_supported=actual_hash == supported_hash,
                       runtime_probe_result="supported" if actual_hash == supported_hash else "RUNTIME_UNSUPPORTED")
    if actual_hash != supported_hash:
        fail("RUNTIME_UNSUPPORTED", "当前微信内核版本尚未适配自动入口，请导出诊断日志用于适配，或使用视觉预约")
