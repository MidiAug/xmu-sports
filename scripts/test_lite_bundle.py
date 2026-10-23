"""Audit the frozen runtime, not merely source import strings. No network/UI."""
import json
from pathlib import Path
import subprocess
import sys

from PyInstaller.archive.readers import CArchiveReader

root = Path(__file__).resolve().parents[1]
config = json.loads((root / "src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
assert config["bundle"]["resources"] == {"resources/http-booking/": "http-booking/"}
assert "ensure-ocr" not in config["build"]["beforeBuildCommand"]
bundle = Path(sys.argv[1]).resolve() if len(sys.argv)>1 else root / "src-tauri/resources/http-booking"
files = [path for path in bundle.rglob("*") if path.is_file()]
for path in files:
    assert not any(part.lower().startswith(("frida", "cv2", "ddddocr", "websockets", "rapidocr", "vision-pack")) for part in path.relative_to(bundle).parts), path
models = [path for path in files if path.suffix == ".onnx"]
assert [p.name for p in models] == ["common_old.onnx"], models
archive = CArchiveReader(str(bundle / "xmu-http-booking.exe")).open_embedded_archive("PYZ.pyz")
for module in archive.toc:
    assert not module.startswith(("frida", "websockets", "cv2", "ddddocr", "xmu_http.runtime_", "xmu_vision")), module
assert "xmu_http.captcha_ocr" in archive.toc
tree = subprocess.run(["cargo", "tree", "--manifest-path", str(root / "src-tauri/Cargo.toml"), "-p", "xmu-sports", "--prefix", "none"], capture_output=True, text=True, check=True)
assert not any(line.startswith("vision-") for line in tree.stdout.splitlines())
report = {"version": config["version"], "resource_files": len(files), "worker_bytes": sum(path.stat().st_size for path in files),
          "only_model": models[0].name, "frozen_python_modules": len(archive.toc), "legacy_modules_absent": True, "host_vision_dependencies_absent": True}
out = root / "artifacts/lite-build/bundle-audit.json"
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps(report, indent=2), encoding="utf-8")
print(json.dumps(report, indent=2))
