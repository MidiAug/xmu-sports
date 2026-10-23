# Vision sidecar (Tauri)

Python **VisionExecutor** (legacy / 对照). Product booking path is in-process
**Native RunSession** (`run_native_loop`); do not add new booking branches here.

Spawned by Rust via:

```text
python -m xmu_vision …
```

cwd = this directory.

```bat
cd src-tauri\sidecars\vision
pip install -r requirements.txt
python -m pytest tests/ -q
python -m xmu_vision doctor
```

Do not import `archive/http-probe`. Shared contract: `docs/contracts/`.
