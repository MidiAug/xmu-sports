import hashlib
import json
import os
import struct
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

from xmu_http import runtime_probe as probe
from xmu_http.session import BookingError


SUPPORTED = b"verified runtime"
SUPPORTED_HASH = hashlib.sha256(SUPPORTED).hexdigest()


@pytest.fixture(autouse=True)
def isolated_native_probe(monkeypatch):
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (None, [], {"image_query": "failed", "image_query_error": 5,
                                                                             "module_query": "failed", "module_query_error": 5}))
    monkeypatch.setattr(probe, "file_version", lambda _: "1.2.3.4")


def process(executable=None, **parameters):
    return SimpleNamespace(pid=987654, parameters={**({"path": str(executable)} if executable else {}), **parameters})


def dll(path, content=SUPPORTED):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)
    return path


def diagnose(process):
    diagnostics = {}
    probe.probe_runtime(process, diagnostics, SUPPORTED_HASH)
    return diagnostics


def test_sibling_dll_and_private_paths(tmp_path):
    directory = tmp_path / "private-user" / "25560"
    dll(directory / "flue.dll")
    result = diagnose(process(directory / "WeChatAppEx.exe", argv=["secret-ticket"]))
    assert result["runtime_supported"] and result["runtime_sha256"] == SUPPORTED_HASH
    assert result["runtime_folder"] == "25560"
    assert result["runtime_dll_source"] == "exe_sibling"
    assert result["runtime_file_version"] == "1.2.3.4"
    assert result["module_query_error"] == 5  # degraded discovery remains diagnosable
    public = json.dumps(result)
    assert "private-user" not in public and str(tmp_path) not in public and "secret-ticket" not in public


def test_numeric_username_is_not_exported_as_runtime_version(tmp_path):
    assert probe.numeric_folder(tmp_path / "22346" / "private-install" / "flue.dll") == "unknown"


@pytest.mark.parametrize("suffix", ["", "x64", "bin/x64"])
def test_version_directory_layout(tmp_path, suffix):
    root = tmp_path / "WMPF"
    dll(root / "25560" / suffix / "flue.dll")
    result = diagnose(process(root / "WeChatAppEx.exe"))
    assert result["runtime_supported"]
    assert result["runtime_dll_source"] == "version_directory"
    assert result["runtime_folder"] == "25560"


def test_radium_directory_layout(tmp_path):
    root = tmp_path / "install"
    dll(root / "RadiumWMPF" / "25560" / "flue.dll")
    assert diagnose(process(root / "WeChatAppEx.exe"))["runtime_supported"]


def test_win32_executable_does_not_depend_on_frida_parameters(monkeypatch, tmp_path):
    path = dll(tmp_path / "25560" / "flue.dll").with_name("WeChatAppEx.exe")
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (path, [], {"image_query": "ok"}))
    result = diagnose(process())
    assert result["runtime_path_source"] == "win32_image" and result["runtime_supported"]


def test_quoted_argv_supports_spaces_and_unicode(tmp_path):
    directory = tmp_path / "微信 安装" / "25560"
    dll(directory / "flue.dll")
    result = diagnose(process(argv=[f'"{directory / "WeChatAppEx.exe"}"', "--secret=ticket"]))
    assert result["runtime_path_source"] == "frida_argv" and result["runtime_supported"]
    assert "ticket" not in json.dumps(result)


def test_relative_or_absent_paths_are_not_resolved_against_worker_directory(tmp_path, monkeypatch):
    dll(tmp_path / "flue.dll")
    monkeypatch.chdir(tmp_path)
    for parameters in ({}, {"argv": []}, {"path": "WeChatAppEx.exe"}):
        with pytest.raises(BookingError) as error:
            diagnose(process(**parameters))
        assert error.value.code == "RUNTIME_PATH_UNAVAILABLE"


def test_missing_dll_is_not_unsupported(tmp_path):
    with pytest.raises(BookingError) as error:
        diagnose(process(tmp_path / "install" / "WeChatAppEx.exe"))
    assert error.value.code == "RUNTIME_FILE_MISSING"
    assert error.value.details["compatibility"]["runtime_candidate_count"] == 0


def test_loaded_module_overrides_supported_sibling_and_old_versions(monkeypatch, tmp_path):
    supported = dll(tmp_path / "25560" / "flue.dll")
    loaded = dll(tmp_path / "25561" / "flue.dll", b"different runtime")
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (supported.with_name("WeChatAppEx.exe"), [loaded], {}))
    with pytest.raises(BookingError) as error:
        diagnose(process())
    assert error.value.code == "RUNTIME_UNSUPPORTED"
    result = error.value.details["compatibility"]
    assert result["runtime_folder"] == "25561" and result["runtime_dll_source"] == "loaded_module"
    assert result["runtime_sha256"] == hashlib.sha256(b"different runtime").hexdigest()


def test_loaded_module_can_be_located_without_executable(monkeypatch, tmp_path):
    loaded = dll(tmp_path / "25560" / "flue.dll")
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (None, [loaded], {}))
    assert diagnose(process())["runtime_supported"]


def test_missing_loaded_module_never_falls_back_to_other_supported_dll(monkeypatch, tmp_path):
    supported = dll(tmp_path / "25560" / "flue.dll")
    monkeypatch.setattr(probe, "native_process_paths", lambda _: (supported.with_name("WeChatAppEx.exe"),
                                                                [tmp_path / "25561" / "flue.dll"], {}))
    with pytest.raises(BookingError) as error:
        diagnose(process())
    assert error.value.code == "RUNTIME_FILE_MISSING"


def test_multiple_versions_never_choose_whichever_hash_is_supported(tmp_path):
    dll(tmp_path / "install" / "25560" / "flue.dll")
    dll(tmp_path / "install" / "25561" / "flue.dll", b"different runtime")
    with pytest.raises(BookingError) as error:
        diagnose(process(tmp_path / "install" / "WeChatAppEx.exe"))
    assert error.value.code == "RUNTIME_AMBIGUOUS"
    assert error.value.details["compatibility"]["runtime_candidate_count"] == 2


def test_unreadable_dll_preserves_numeric_error_without_exporting_path(monkeypatch, tmp_path):
    path = dll(tmp_path / "private-user" / "25560" / "flue.dll")
    original = Path.open
    def read(self, *args, **kwargs):
        if self == path:
            raise PermissionError(13, "private path and secret", str(path))
        return original(self, *args, **kwargs)
    monkeypatch.setattr(Path, "open", read)
    with pytest.raises(BookingError) as error:
        diagnose(process(path.with_name("WeChatAppEx.exe")))
    assert error.value.code == "RUNTIME_FILE_UNREADABLE"
    assert error.value.details["compatibility"]["runtime_file_error"] == 13
    assert "private" not in json.dumps(error.value.public())


def test_large_directory_search_is_bounded_and_does_not_guess(tmp_path):
    root = tmp_path / "install"
    root.mkdir()
    for i in range(130):
        (root / f"other-{i}").mkdir()
    dll(root / "25560" / "flue.dll")
    with pytest.raises(BookingError) as error:
        diagnose(process(root / "WeChatAppEx.exe"))
    assert error.value.code == "RUNTIME_AMBIGUOUS"
    assert error.value.details["compatibility"]["layout_search_truncated"]


@pytest.mark.parametrize("machine,name", [(0x8664, "x64"), (0x14c, "x86"), (0xaa64, "arm64")])
def test_pe_architecture(tmp_path, machine, name):
    header = bytearray(64)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 60, 64)
    path = dll(tmp_path / "flue.dll", header + b"PE\0\0" + struct.pack("<H", machine))
    assert probe.pe_architecture(path) == name


@pytest.mark.skipif(os.name != "nt", reason="Win32 API smoke check")
def test_native_api_reads_current_process_without_attach(monkeypatch):
    # Undo fixture stubs to exercise actual ctypes signatures on Windows.
    monkeypatch.undo()
    executable, modules, diagnostics = probe.native_process_paths(os.getpid())
    # A Windows venv launcher may run the base interpreter as the process image.
    images = [Path(sys.executable), Path(getattr(sys, "_base_executable", sys.executable))]
    assert executable and any(executable.samefile(image) for image in images)
    assert diagnostics["image_query"] == "ok" and diagnostics["module_query"] == "ok"
    assert diagnostics["module_count"] > 0 and modules == []
    assert probe.file_version(executable) is not None
