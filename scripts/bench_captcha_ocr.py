#!/usr/bin/env python3
"""Benchmark captcha OCR engines on scripts/fixtures/captcha-ocr/*.png.

Ground truth is encoded in the filename: NN_ANSWER.png (or ANSWER.png).

Engines (each skipped if import fails):
  - ddddocr beta / old
  - ppllocr (optional: pip install ppllocr)
  - rapidocr_onnxruntime (optional)
  - native Rust path via cargo run -p vision-platform-windows --bin vision-captcha-diag

Usage:
  python scripts/bench_captcha_ocr.py
  python scripts/bench_captcha_ocr.py --native-only
  python scripts/bench_captcha_ocr.py --no-native
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures" / "captcha-ocr"
ANSWER_RE = re.compile(r"(?:^|_)([A-Za-z0-9]{4})\.png$", re.I)


def load_cases(dir_path: Path) -> list[tuple[str, Path]]:
    cases: list[tuple[str, Path]] = []
    for p in sorted(dir_path.glob("*.png")):
        m = ANSWER_RE.search(p.name)
        if not m:
            print(f"[skip] no answer in name: {p.name}", file=sys.stderr)
            continue
        cases.append((m.group(1).upper(), p))
    return cases


def norm(text: str | None) -> str:
    if not text:
        return ""
    return "".join(c for c in text if c.isalnum()).upper()[:8]


def score(pred: str, expect: str) -> str:
    p = norm(pred)
    if p == expect:
        return "OK"
    if len(p) == 4 and p:
        return f"WRONG({p})"
    if p:
        return f"SHORT({p})"
    return "EMPTY"


# ----- engines -----


def eng_ddddocr(beta: bool):
    import ddddocr

    ocr = ddddocr.DdddOcr(show_ad=False, beta=beta)
    label = f"ddddocr_{'beta' if beta else 'old'}"

    def run(path: Path) -> str:
        return ocr.classification(path.read_bytes()) or ""

    return label, run


def eng_ppllocr():
    import ppllocr

    ocr = ppllocr.OCR()
    label = "ppllocr"

    def run(path: Path) -> str:
        return ocr.classification(path.read_bytes()) or ""

    return label, run


def eng_rapidocr():
    from rapidocr_onnxruntime import RapidOCR

    engine = RapidOCR()
    label = "rapidocr"

    def run(path: Path) -> str:
        result, _ = engine(str(path))
        if not result:
            return ""
        return "".join(row[1] for row in result)

    return label, run


def try_engine(factory):
    try:
        return factory()
    except Exception as e:  # noqa: BLE001
        print(f"[skip engine] {factory.__name__ if hasattr(factory, '__name__') else factory}: {e}")
        return None


def run_python_engines(cases: list[tuple[str, Path]]) -> None:
    engines = []
    for factory in (
        lambda: eng_ddddocr(True),
        lambda: eng_ddddocr(False),
        eng_ppllocr,
        eng_rapidocr,
    ):
        got = try_engine(factory)
        if got:
            engines.append(got)

    if not engines:
        print("No Python OCR engines available.")
        return

    print("\n=== Python engines ===")
    header = f"{'file':16} {'expect':6}" + "".join(f"  {name:16}" for name, _ in engines)
    print(header)
    totals = {name: 0 for name, _ in engines}
    for expect, path in cases:
        cells = []
        for name, fn in engines:
            t0 = time.perf_counter()
            try:
                raw = fn(path)
            except Exception as e:  # noqa: BLE001
                raw = f"ERR:{e}"
            ms = (time.perf_counter() - t0) * 1000
            mark = score(raw, expect)
            if mark == "OK":
                totals[name] += 1
            cells.append(f"{mark}@{ms:.0f}ms".ljust(16))
        print(f"{path.name:16} {expect:6}" + "  ".join(cells))
    n = len(cases)
    print("---")
    for name, _ in engines:
        print(f"  {name}: {totals[name]}/{n}")


def run_native(cases: list[tuple[str, Path]], cargo_manifest: Path) -> None:
    print("\n=== Native Rust (recognize_captcha_bgra) ===")
    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "vision-platform-windows",
        "--bin",
        "vision-captcha-diag",
        "--features",
        "diag-bin",
        "--",
        str(FIXTURES),
    ]
    print(">", " ".join(cmd))
    t0 = time.perf_counter()
    proc = subprocess.run(
        cmd,
        cwd=str(cargo_manifest.parent),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    ms = (time.perf_counter() - t0) * 1000
    sys.stdout.write(proc.stdout or "")
    if proc.stderr:
        # cargo/build noise often lands here; keep last useful lines
        err_lines = [ln for ln in proc.stderr.splitlines() if ln.strip()]
        for ln in err_lines[-20:]:
            print(f"[native-stderr] {ln}")
    if proc.returncode != 0:
        print(f"native exit={proc.returncode} wall={ms:.0f}ms")
    else:
        print(f"native wall={ms:.0f}ms (includes compile if cold)")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--no-native", action="store_true", help="skip Rust diag bin")
    ap.add_argument("--native-only", action="store_true", help="only Rust diag bin")
    ap.add_argument(
        "--fixtures",
        type=Path,
        default=FIXTURES,
        help="directory of NN_ANSWER.png fixtures",
    )
    args = ap.parse_args()

    cases = load_cases(args.fixtures)
    if not cases:
        print(f"No fixtures in {args.fixtures}", file=sys.stderr)
        return 1
    print(f"fixtures={args.fixtures} n={len(cases)}")
    for expect, path in cases:
        print(f"  {path.name} -> {expect}")

    if not args.native_only:
        run_python_engines(cases)
    if not args.no_native:
        cargo_toml = ROOT / "src-tauri" / "Cargo.toml"
        run_native(cases, cargo_toml)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
