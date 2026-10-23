"""Check tracked publication boundaries and application version consistency (offline)."""
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def check():
    files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
    blocked_prefixes = ("docs/", "images/", "captures/", "archive/", "reference/", "artifacts/", ".cursor/", ".agents/", ".claude/", ".codex/", ".codegraph/", "src-tauri/sidecars/vision/fixtures/")
    violations = []
    for name in filter(None, files):
        p = Path(name)
        if (name.startswith(blocked_prefixes) or p.name in {"AGENTS.md", "CLAUDE.md", "GEMINI.md", ".env", "codegraph.json", ".cursorignore", ".codegraphignore"}
            or p.name.startswith(".env.") or p.suffix.lower() in {".db", ".sqlite", ".sqlite3", ".har", ".saz", ".pem", ".key", ".log"}
            or "node_modules" in p.parts or ".venv" in p.parts or "target" in p.parts):
            violations.append(name)
    assert not violations, "Private/generated paths tracked: " + ", ".join(violations)
    package = json.loads((ROOT / "package.json").read_text())
    lock = json.loads((ROOT / "package-lock.json").read_text())
    tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    cargo = tomllib.loads((ROOT / "src-tauri/Cargo.toml").read_text())
    cargo_lock = tomllib.loads((ROOT / "src-tauri/Cargo.lock").read_text())
    rust_version = next(p["version"] for p in cargo_lock["package"] if p["name"] == "xmu-sports")
    versions = {package["version"], lock["version"], lock["packages"][""]["version"], tauri["version"], cargo["package"]["version"], rust_version}
    assert len(versions) == 1, f"Version mismatch: {versions}"
    print(f"Public tree boundaries OK; application version {package['version']}")
    return package["version"]

if __name__ == "__main__":
    check()
