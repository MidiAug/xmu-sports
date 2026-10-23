#!/usr/bin/env python3
"""Generate captcha enhancements and score them with ddddocr + native Rust.

Writes enhanced PNGs under artifacts/captcha-enhance/<variant>/NN_ANSWER.png
then runs recognition. Does not change product OCR code.

Usage:
  python scripts/bench_captcha_enhance.py
  python scripts/bench_captcha_enhance.py --no-native
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import time
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "scripts" / "fixtures" / "captcha-ocr"
OUT = ROOT / "artifacts" / "captcha-enhance"
ANSWER_RE = re.compile(r"(?i)(?:^|_)([A-Za-z0-9]{4})\.png$")


def cases(dir_path: Path) -> list[tuple[str, Path]]:
    out = []
    for p in sorted(dir_path.glob("*.png")):
        m = ANSWER_RE.search(p.name)
        if m:
            out.append((m.group(1).upper(), p))
    return out


def norm(s: str | None) -> str:
    return "".join(c for c in (s or "") if c.isalnum()).upper()[:8]


# ----- enhancements (BGR in/out) -----


def enh_raw(bgr: np.ndarray) -> np.ndarray:
    return bgr


def enh_up2(bgr: np.ndarray) -> np.ndarray:
    return cv2.resize(bgr, None, fx=2.0, fy=2.0, interpolation=cv2.INTER_CUBIC)


def enh_clahe(bgr: np.ndarray) -> np.ndarray:
    lab = cv2.cvtColor(bgr, cv2.COLOR_BGR2LAB)
    l, a, b = cv2.split(lab)
    clahe = cv2.createCLAHE(clipLimit=2.0, tileGridSize=(4, 4))
    l2 = clahe.apply(l)
    return cv2.cvtColor(cv2.merge([l2, a, b]), cv2.COLOR_LAB2BGR)


def enh_otsu(bgr: np.ndarray) -> np.ndarray:
    gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
    _, thr = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY + cv2.THRESH_OTSU)
    return cv2.cvtColor(thr, cv2.COLOR_GRAY2BGR)


def enh_otsu_inv(bgr: np.ndarray) -> np.ndarray:
    gray = cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)
    _, thr = cv2.threshold(gray, 0, 255, cv2.THRESH_BINARY_INV + cv2.THRESH_OTSU)
    return cv2.cvtColor(thr, cv2.COLOR_GRAY2BGR)


def enh_ink_mask(bgr: np.ndarray) -> np.ndarray:
    """Keep dark / saturated ink; wipe pale-blue noise background to white."""
    hsv = cv2.cvtColor(bgr, cv2.COLOR_BGR2HSV)
    h, s, v = cv2.split(hsv)
    # Pale paper / noise: high V + low S
    pale = (v >= 170) & (s <= 70)
    # Also wipe very light gray noise dots
    pale |= (v >= 200) & (s <= 100)
    out = bgr.copy()
    out[pale] = (255, 255, 255)
    return out


def enh_ink_then_clahe(bgr: np.ndarray) -> np.ndarray:
    return enh_clahe(enh_ink_mask(bgr))


def enh_ink_up2(bgr: np.ndarray) -> np.ndarray:
    return enh_up2(enh_ink_mask(bgr))


def enh_denoise(bgr: np.ndarray) -> np.ndarray:
    return cv2.fastNlMeansDenoisingColored(bgr, None, 6, 6, 5, 15)


def enh_sharpen(bgr: np.ndarray) -> np.ndarray:
    blur = cv2.GaussianBlur(bgr, (0, 0), 1.0)
    return cv2.addWeighted(bgr, 1.6, blur, -0.6, 0)


ENHANCERS: list[tuple[str, callable]] = [
    ("00_raw", enh_raw),
    ("01_up2", enh_up2),
    ("02_clahe", enh_clahe),
    ("03_otsu", enh_otsu),
    ("04_otsu_inv", enh_otsu_inv),
    ("05_ink_mask", enh_ink_mask),
    ("06_ink_clahe", enh_ink_then_clahe),
    ("07_ink_up2", enh_ink_up2),
    ("08_denoise", enh_denoise),
    ("09_sharpen", enh_sharpen),
]


def write_variants(src_cases: list[tuple[str, Path]]) -> dict[str, Path]:
    OUT.mkdir(parents=True, exist_ok=True)
    dirs: dict[str, Path] = {}
    for name, fn in ENHANCERS:
        d = OUT / name
        d.mkdir(parents=True, exist_ok=True)
        dirs[name] = d
        for expect, path in src_cases:
            bgr = cv2.imread(str(path), cv2.IMREAD_COLOR)
            if bgr is None:
                raise RuntimeError(f"imread failed: {path}")
            out = fn(bgr)
            # Keep answer in filename for native diag.
            dest = d / f"{path.stem.split('_')[0] if '_' in path.stem else '00'}_{expect}.png"
            # Prefer original stem style: 01_6ZVT.png
            dest = d / path.name
            cv2.imwrite(str(dest), out)
    return dirs


def score_ddddocr(dirs: dict[str, Path], src_cases: list[tuple[str, Path]]) -> None:
    import ddddocr

    engines = [
        ("beta", ddddocr.DdddOcr(show_ad=False, beta=True)),
        ("old", ddddocr.DdddOcr(show_ad=False, beta=False)),
    ]
    # Also try Python color filter on raw
    print("\n=== ddddocr on enhanced variants ===")
    header = f"{'variant':14}" + "".join(f" {lab:>8}" for lab, _ in engines)
    print(header)
    best = {lab: (0, "") for lab, _ in engines}
    n = len(src_cases)
    for vname, d in dirs.items():
        cells = []
        for lab, ocr in engines:
            hit = 0
            for expect, path in src_cases:
                p = d / path.name
                raw = ocr.classification(p.read_bytes()) or ""
                if norm(raw) == expect:
                    hit += 1
            cells.append(f"{hit}/{n}")
            if hit > best[lab][0]:
                best[lab] = (hit, vname)
        print(f"{vname:14}" + "".join(f" {c:>8}" for c in cells))
    print("best:", {k: f"{v[0]}/{n} @{v[1]}" for k, v in best.items()})

    # Color-filter on raw originals (Python API)
    print("\n=== ddddocr beta + color_filter on RAW ===")
    ocr = ddddocr.DdddOcr(show_ad=False, beta=True)
    for colors in (
        ["black"],
        ["blue"],
        ["purple"],
        ["green"],
        ["black", "blue", "purple", "green", "yellow", "orange"],
    ):
        hit = 0
        detail = []
        for expect, path in src_cases:
            try:
                raw = ocr.classification(path.read_bytes(), color_filter_colors=colors) or ""
            except TypeError:
                # older API name
                try:
                    raw = ocr.classification(path.read_bytes(), colors=colors) or ""
                except Exception as e:  # noqa: BLE001
                    print(f"  color_filter unsupported: {e}")
                    return
            except Exception as e:  # noqa: BLE001
                raw = f"ERR:{e}"
            mark = "OK" if norm(raw) == expect else f"{norm(raw) or 'EMPTY'}"
            if mark == "OK":
                hit += 1
            detail.append(f"{expect}->{mark}")
        print(f"  colors={colors}: {hit}/{n}  ({', '.join(detail)})")


def score_native(dirs: dict[str, Path]) -> None:
    print("\n=== native Rust on enhanced variants ===")
    cargo = ROOT / "src-tauri"
    for vname, d in dirs.items():
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
            str(d),
        ]
        t0 = time.perf_counter()
        proc = subprocess.run(
            cmd,
            cwd=str(cargo),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        ms = (time.perf_counter() - t0) * 1000
        summary = ""
        for ln in (proc.stdout or "").splitlines():
            if ln.startswith("summary"):
                summary = ln
        status = "ok" if proc.returncode == 0 else f"exit={proc.returncode}"
        print(f"  {vname:14} {summary or '(no summary)'} {status} wall={ms:.0f}ms")
        # Print misses for interesting variants
        if vname in ("00_raw", "05_ink_mask", "06_ink_clahe", "07_ink_up2"):
            for ln in (proc.stdout or "").splitlines():
                if "mark=WRONG" in ln or "mark=SHORT" in ln or "raw=" in ln and "mark=OK" not in ln:
                    if "mark=" in ln:
                        print(f"    {ln}")


def score_native_strategies(fixtures: Path) -> None:
    print("\n=== native Rust strategies on RAW fixtures ===")
    cargo = ROOT / "src-tauri"
    for strategy in ("default", "alnum", "ink_colors", "no_upscale"):
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
            "--strategy",
            strategy,
            str(fixtures),
        ]
        proc = subprocess.run(
            cmd,
            cwd=str(cargo),
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        summary = next(
            (ln for ln in (proc.stdout or "").splitlines() if ln.startswith("summary")),
            "(no summary)",
        )
        print(f"  strategy={strategy:12} {summary}")
        for ln in (proc.stdout or "").splitlines():
            if "mark=WRONG" in ln or "mark=SHORT" in ln:
                print(f"    {ln}")
        if proc.returncode not in (0, 1) and proc.stderr:
            err = [ln for ln in proc.stderr.splitlines() if "error" in ln.lower()][-5:]
            for ln in err:
                print(f"    [err] {ln}")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--no-native", action="store_true")
    ap.add_argument("--fixtures", type=Path, default=FIXTURES)
    args = ap.parse_args()

    src = cases(args.fixtures)
    if not src:
        print(f"no fixtures in {args.fixtures}", file=sys.stderr)
        return 1
    print(f"n={len(src)} fixtures → enhance under {OUT}")
    dirs = write_variants(src)
    score_ddddocr(dirs, src)
    if not args.no_native:
        score_native_strategies(args.fixtures)
        score_native(dirs)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
