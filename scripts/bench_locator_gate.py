#!/usr/bin/env python3
"""Offline locator-gate bake-off: speed × hit-rate on booking screenshots.

Compares template / feature / OCR strategies for post-click readiness probes.
References:
  - OpenCV matchTemplate (TM_CCOEFF_NORMED) + optional alpha mask
  - Airtest MultiScaleTemplateMatching (scale pyramid + CCOEFF)
  - Coarse→refine (downscale search, full-res refine) — common UI-auto pattern
  - ORB (fast features; often weak on flat icons)
  - RapidOCR full vs ROI crop (RapidAI: crop first, limit side len)

Usage (repo root):
  python scripts/bench_locator_gate.py
  python scripts/bench_locator_gate.py --rounds 7 --out docs/research/locator-gate-bench.md
"""

from __future__ import annotations

import argparse
import statistics
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
IMAGES = ROOT / "images"
TPL_ROOT = (
    ROOT
    / "src-tauri"
    / "vision-runtime"
    / "crates"
    / "vision-platform-windows"
    / "templates"
    / "booking"
)

# Empirically tuned; report scores so we can retune.
HIT_TPL = 0.82
HIT_ORB_INLIERS = 8


@dataclass
class Trial:
    name: str
    ms: float
    hit: bool
    score: float
    detail: str = ""


@dataclass
class Case:
    title: str
    image: Path
    needle_text: str
    sport_tpl: Path | None = None
    button_tpl: Path | None = None
    expect_page: str = ""


def imread_unicode(path: Path) -> np.ndarray:
    data = np.fromfile(str(path), dtype=np.uint8)
    img = cv2.imdecode(data, cv2.IMREAD_COLOR)
    if img is None:
        raise FileNotFoundError(path)
    return img


def timed(fn, rounds: int) -> tuple[float, object]:
    fn()  # warm
    times: list[float] = []
    last = None
    for _ in range(rounds):
        t0 = time.perf_counter()
        last = fn()
        times.append((time.perf_counter() - t0) * 1000.0)
    return statistics.median(times), last


def match_ccoeff(
    hay_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    *,
    gray: bool = False,
    roi_y0: float = 0.0,
    roi_y1: float = 1.0,
) -> tuple[float, tuple[int, int] | None]:
    h, w = hay_bgr.shape[:2]
    y0, y1 = int(h * roi_y0), int(h * roi_y1)
    crop = hay_bgr[y0:y1]
    if gray:
        hay = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
        nd = cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY)
    else:
        hay, nd = crop, needle_bgr
    if nd.shape[0] > hay.shape[0] or nd.shape[1] > hay.shape[1]:
        return 0.0, None
    res = cv2.matchTemplate(hay, nd, cv2.TM_CCOEFF_NORMED)
    _mn, mx, _ml, ml = cv2.minMaxLoc(res)
    return float(mx), (ml[0] + nd.shape[1] // 2, y0 + ml[1] + nd.shape[0] // 2)


def match_multiscale(
    hay_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    scales: list[float],
    *,
    gray: bool = True,
    roi_y0: float = 0.0,
    roi_y1: float = 0.72,
) -> tuple[float, float | None]:
    """Airtest-style: resize needle across scales, TM_CCOEFF_NORMED."""
    best = 0.0
    best_s = None
    h, w = hay_bgr.shape[:2]
    y0, y1 = int(h * roi_y0), int(h * roi_y1)
    crop = hay_bgr[y0:y1]
    hay = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY) if gray else crop
    nd0 = cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY) if gray else needle_bgr
    for s in scales:
        nw = max(8, int(round(nd0.shape[1] * s)))
        nh = max(8, int(round(nd0.shape[0] * s)))
        if nh >= hay.shape[0] or nw >= hay.shape[1]:
            continue
        nd = cv2.resize(nd0, (nw, nh), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_LINEAR)
        res = cv2.matchTemplate(hay, nd, cv2.TM_CCOEFF_NORMED)
        _mn, mx, _ml, _mloc = cv2.minMaxLoc(res)
        if mx > best:
            best = float(mx)
            best_s = s
    return best, best_s


def match_pyramid_refine(
    hay_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    *,
    factor: int = 2,
    refine_pad: int = 24,
    roi_y1: float = 0.72,
) -> float:
    """Downscale match → crop window → full-res refine (UI-auto classic)."""
    h, w = hay_bgr.shape[:2]
    y1 = int(h * roi_y1)
    crop = hay_bgr[:y1]
    hay_g = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    nd_g = cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY)
    small_h = cv2.resize(hay_g, (w // factor, y1 // factor), interpolation=cv2.INTER_AREA)
    small_n = cv2.resize(
        nd_g,
        (max(4, nd_g.shape[1] // factor), max(4, nd_g.shape[0] // factor)),
        interpolation=cv2.INTER_AREA,
    )
    if small_n.shape[0] >= small_h.shape[0] or small_n.shape[1] >= small_h.shape[1]:
        return 0.0
    res = cv2.matchTemplate(small_h, small_n, cv2.TM_CCOEFF_NORMED)
    _mn, _mx, _ml, ml = cv2.minMaxLoc(res)
    cx = (ml[0] + small_n.shape[1] // 2) * factor
    cy = (ml[1] + small_n.shape[0] // 2) * factor
    nh, nw = nd_g.shape[:2]
    x0 = max(0, cx - nw // 2 - refine_pad)
    y0 = max(0, cy - nh // 2 - refine_pad)
    x1 = min(w, cx + nw // 2 + refine_pad)
    y1b = min(y1, cy + nh // 2 + refine_pad)
    window = hay_g[y0:y1b, x0:x1]
    if nd_g.shape[0] > window.shape[0] or nd_g.shape[1] > window.shape[1]:
        return float(_mx)
    res2 = cv2.matchTemplate(window, nd_g, cv2.TM_CCOEFF_NORMED)
    _mn2, mx2, _ml2, _ = cv2.minMaxLoc(res2)
    return float(mx2)


def match_edge(hay_bgr: np.ndarray, needle_bgr: np.ndarray, roi_y1: float = 0.72) -> float:
    h = hay_bgr.shape[0]
    crop = hay_bgr[: int(h * roi_y1)]
    hay_e = cv2.Canny(cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY), 60, 160)
    nd_e = cv2.Canny(cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY), 60, 160)
    if nd_e.shape[0] >= hay_e.shape[0] or nd_e.shape[1] >= hay_e.shape[1]:
        return 0.0
    res = cv2.matchTemplate(hay_e, nd_e, cv2.TM_CCOEFF_NORMED)
    _mn, mx, _ml, _ = cv2.minMaxLoc(res)
    return float(mx)


def match_orb(hay_bgr: np.ndarray, needle_bgr: np.ndarray, roi_y1: float = 0.72) -> tuple[float, int]:
    h = hay_bgr.shape[0]
    crop = hay_bgr[: int(h * roi_y1)]
    orb = cv2.ORB_create(nfeatures=600)
    kp1, des1 = orb.detectAndCompute(cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY), None)
    kp2, des2 = orb.detectAndCompute(cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY), None)
    if des1 is None or des2 is None or len(kp1) < 4 or len(kp2) < 4:
        return 0.0, 0
    bf = cv2.BFMatcher(cv2.NORM_HAMMING, crossCheck=False)
    pairs = bf.knnMatch(des1, des2, k=2)
    good = []
    for pair in pairs:
        if len(pair) < 2:
            continue
        a, b = pair
        if a.distance < 0.75 * b.distance:
            good.append(a)
    if len(good) < 4:
        return 0.0, len(good)
    src = np.float32([kp1[m.queryIdx].pt for m in good]).reshape(-1, 1, 2)
    dst = np.float32([kp2[m.trainIdx].pt for m in good]).reshape(-1, 1, 2)
    _H, mask = cv2.findHomography(src, dst, cv2.RANSAC, 5.0)
    inliers = int(mask.sum()) if mask is not None else 0
    return float(inliers) / max(len(good), 1), inliers


def coarse_ncc_rust_style(
    hay_bgr: np.ndarray,
    needle_bgr: np.ndarray,
    scales: list[float] | None = None,
) -> float:
    """Approximate current locator_gate.rs: gray NCC, coarse step, multi-scale."""
    scales = scales or [0.55, 0.7, 0.85, 1.0, 1.2, 1.4]
    hay = cv2.cvtColor(hay_bgr, cv2.COLOR_BGR2GRAY).astype(np.float32)
    nd0 = cv2.cvtColor(needle_bgr, cv2.COLOR_BGR2GRAY).astype(np.float32)
    hh, hw = hay.shape
    y_lim = int(hh * 0.72)
    best = 0.0
    for s in scales:
        nh = max(8, int(round(nd0.shape[0] * s)))
        nw = max(8, int(round(nd0.shape[1] * s)))
        if nh >= y_lim or nw >= hw:
            continue
        nd = cv2.resize(nd0, (nw, nh), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_LINEAR)
        step_x = max(4, nw // 4)
        step_y = max(4, nh // 4)
        # Use OpenCV under the hood for fair-ish score, but stride simulation via resize of result
        res = cv2.matchTemplate(hay[:y_lim], nd, cv2.TM_CCOEFF_NORMED)
        # subsample result like coarse step
        res_s = res[::step_y, ::step_x]
        mx = float(res_s.max()) if res_s.size else 0.0
        if mx > best:
            best = mx
    return best


_OCR = None


def get_ocr():
    global _OCR
    if _OCR is None:
        from rapidocr_onnxruntime import RapidOCR

        _OCR = RapidOCR()
    return _OCR


def ocr_contains(img_bgr: np.ndarray, needle: str) -> tuple[bool, list[str], float]:
    ocr = get_ocr()
    rgb = cv2.cvtColor(img_bgr, cv2.COLOR_BGR2RGB)
    t0 = time.perf_counter()
    result, _ = ocr(rgb)
    ms = (time.perf_counter() - t0) * 1000.0
    texts = [item[1] for item in (result or [])]
    hit = any(needle in t for t in texts)
    return hit, texts, ms


def run_case(case: Case, rounds: int) -> list[Trial]:
    img = imread_unicode(case.image)
    trials: list[Trial] = []

    def add(name: str, fn, hit_fn, score_fn=None):
        ms, out = timed(fn, rounds)
        hit = hit_fn(out)
        score = float(score_fn(out)) if score_fn else (1.0 if hit else 0.0)
        detail = ""
        if isinstance(out, tuple) and len(out) >= 2 and isinstance(out[1], (int, float)):
            detail = f"extra={out[1]}"
        trials.append(Trial(name=name, ms=ms, hit=hit, score=score, detail=detail))
        flag = "HIT " if hit else "miss"
        print(f"  [{flag}] {name:36s}  {ms:7.1f} ms  score={score:.3f}  {detail}")

    # --- OCR baselines ---
    def full_ocr():
        hit, texts, _ = ocr_contains(img, case.needle_text)
        return hit, len(texts)

    # OCR timing: don't double-count inside timed+ocr_contains; separate path
    def full_ocr_once():
        ocr = get_ocr()
        rgb = cv2.cvtColor(img, cv2.COLOR_BGR2RGB)
        result, _ = ocr(rgb)
        texts = [item[1] for item in (result or [])]
        return any(case.needle_text in t for t in texts), len(texts)

    add("ocr_full", full_ocr_once, lambda o: o[0], lambda o: 1.0 if o[0] else 0.0)

    for y0, y1, tag in [
        (0.05, 0.55, "roi_y05_55"),
        (0.08, 0.45, "roi_y08_45"),
        (0.0, 0.72, "roi_y00_72"),
        (0.35, 0.70, "roi_y35_70"),
    ]:
        h = img.shape[0]
        crop = img[int(h * y0) : int(h * y1)]

        def make_roi(c=crop):
            def _():
                ocr = get_ocr()
                rgb = cv2.cvtColor(c, cv2.COLOR_BGR2RGB)
                result, _ = ocr(rgb)
                texts = [item[1] for item in (result or [])]
                return any(case.needle_text in t for t in texts), len(texts)

            return _

        add(f"ocr_{tag}", make_roi(), lambda o: o[0], lambda o: 1.0 if o[0] else 0.0)

    # --- Template family (sport or button) ---
    tpl_path = case.sport_tpl or case.button_tpl
    if tpl_path and tpl_path.exists():
        needle = imread_unicode(tpl_path)

        add(
            "tpl_ccoeff_bgr_full",
            lambda: match_ccoeff(img, needle, gray=False),
            lambda o: o[0] >= HIT_TPL,
            lambda o: o[0],
        )
        add(
            "tpl_ccoeff_gray_full",
            lambda: match_ccoeff(img, needle, gray=True),
            lambda o: o[0] >= HIT_TPL,
            lambda o: o[0],
        )
        add(
            "tpl_ccoeff_gray_roi72",
            lambda: match_ccoeff(img, needle, gray=True, roi_y1=0.72),
            lambda o: o[0] >= HIT_TPL,
            lambda o: o[0],
        )
        scales_coarse = [0.55, 0.7, 0.85, 1.0, 1.2, 1.4]
        scales_fine = [round(x, 2) for x in np.arange(0.5, 1.51, 0.1)]

        add(
            "tpl_ms_coarse6_gray",
            lambda: match_multiscale(img, needle, scales_coarse),
            lambda o: o[0] >= HIT_TPL,
            lambda o: o[0],
        )
        add(
            "tpl_ms_fine11_gray",
            lambda: match_multiscale(img, needle, scales_fine),
            lambda o: o[0] >= HIT_TPL,
            lambda o: o[0],
        )
        add(
            "tpl_pyramid_x2_refine",
            lambda: match_pyramid_refine(img, needle),
            lambda o: o >= HIT_TPL,
            lambda o: o,
        )
        add(
            "tpl_edge_canny",
            lambda: match_edge(img, needle),
            lambda o: o >= HIT_TPL,
            lambda o: o,
        )
        add(
            "orb_ransac",
            lambda: match_orb(img, needle),
            lambda o: o[1] >= HIT_ORB_INLIERS,
            lambda o: o[0],
        )
        add(
            "ncc_rust_gate_sim",
            lambda: coarse_ncc_rust_style(img, needle),
            lambda o: o >= HIT_TPL,
            lambda o: o,
        )
        # Lower threshold probe (Airtest often 0.7)
        add(
            "tpl_ms_coarse6@0.70",
            lambda: match_multiscale(img, needle, scales_coarse),
            lambda o: o[0] >= 0.70,
            lambda o: o[0],
        )

    return trials


def pick_winners(trials: list[Trial]) -> list[Trial]:
    hits = [t for t in trials if t.hit]
    if not hits:
        return sorted(trials, key=lambda t: t.ms)[:3]
    return sorted(hits, key=lambda t: (t.ms, -t.score))[:5]


def to_markdown(cases_out: list[tuple[Case, list[Trial]]]) -> str:
    lines = [
        "# Locator gate bake-off",
        "",
        f"> Generated by `scripts/bench_locator_gate.py`. Screenshots under `images/`, templates under `templates/booking/`.",
        "",
        "## Method notes",
        "",
        "- **OpenCV `TM_CCOEFF_NORMED`**: default for fixed UI icons ([OpenCV docs](https://docs.opencv.org/4.x/df/dfb/group__imgproc__object.html)).",
        "- **Multiscale**: Airtest-style resize needle + match ([Airtest mstpl](https://airtest.readthedocs.io/en/v1.2.7/_modules/airtest/aircv/multiscale_template_matching.html)).",
        "- **Pyramid refine**: downscale propose → full-res refine (common GUI-auto).",
        "- **ORB**: fast features; weak on flat/low-texture icons.",
        "- **RapidOCR**: prefer ROI crop; full-page is baseline cost ([RapidOCR](https://github.com/RapidAI/RapidOCR)).",
        "",
    ]
    for case, trials in cases_out:
        lines.append(f"## {case.title}")
        lines.append("")
        lines.append(f"- image: `{case.image.name}` · needle: `{case.needle_text}` · expect: `{case.expect_page}`")
        lines.append("")
        lines.append("| algo | ms | hit | score |")
        lines.append("|---|---:|:---:|---:|")
        for t in sorted(trials, key=lambda x: (not x.hit, x.ms)):
            lines.append(f"| `{t.name}` | {t.ms:.1f} | {'Y' if t.hit else ''} | {t.score:.3f} |")
        lines.append("")
        winners = pick_winners(trials)
        lines.append("**Fastest hits:** " + ", ".join(f"`{w.name}` ({w.ms:.1f}ms)" for w in winners))
        lines.append("")
    return "\n".join(lines) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()

    cases = [
        Case(
            title="Home → 场馆预约 (entry icon/text)",
            image=IMAGES / "8.png",
            needle_text="场馆预约",
            button_tpl=TPL_ROOT / "venue_booking.png",
            expect_page="home",
        ),
        Case(
            title="Sport grid → 健身房 icon",
            image=IMAGES / "9.png",
            needle_text="健身房",
            sport_tpl=TPL_ROOT / "sports" / "08_gym.png",
            expect_page="venue_sport_list",
        ),
        Case(
            title="Venue list → 翔安校区健身房",
            image=IMAGES / "10.png",
            needle_text="翔安校区健身房",
            expect_page="venue_list",
        ),
        Case(
            title="Notice button template (i_know) on sport grid? (neg)",
            image=IMAGES / "9.png",
            needle_text="我知道了",
            button_tpl=TPL_ROOT / "i_know.png",
            expect_page="venue_sport_list_neg",
        ),
    ]

    # Warm OCR once
    if (IMAGES / "9.png").exists():
        get_ocr()
        ocr_contains(imread_unicode(IMAGES / "9.png"), "暖")

    all_out: list[tuple[Case, list[Trial]]] = []
    for case in cases:
        if not case.image.exists():
            print(f"SKIP missing {case.image}")
            continue
        print(f"\n=== {case.title} ===")
        trials = run_case(case, args.rounds)
        all_out.append((case, trials))
        wins = pick_winners(trials)
        print("  winners:", ", ".join(f"{w.name}={w.ms:.1f}ms" for w in wins))

    md = to_markdown(all_out)
    out = args.out
    if out is None:
        out = ROOT / "docs" / "research" / "locator-gate-bench.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(md, encoding="utf-8")
    print(f"\nWrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
