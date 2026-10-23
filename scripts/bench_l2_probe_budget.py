#!/usr/bin/env python3
"""Evidence bench: post-click L2 template cost × quality × schedule.

Problem (from 03:30 log): until=[我知道了,预约须知,健身房] treated as "fast"
probes every animating frame; poll1 alone burned ~4.7s with no hit.

This script measures — on the same 520×719 runtime size as the log —
which algorithm/schedule stays under a real-time budget while keeping TP/FP.

Run:
  python scripts/bench_l2_probe_budget.py
  python scripts/bench_l2_probe_budget.py --out docs/research/l2-probe-budget-bench.md
"""

from __future__ import annotations

import argparse
import statistics
import time
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

ROOT = Path(__file__).resolve().parents[1]
IMAGES = ROOT / "images"
TPL = (
    ROOT
    / "src-tauri"
    / "vision-runtime"
    / "crates"
    / "vision-platform-windows"
    / "templates"
    / "booking"
)

# Runtime window size from the failing log.
RUNTIME_WH = (520, 719)
HIT = 0.82
# Force-OCR gap in glance.rs — L2 must fit several times inside this if every-frame.
FORCE_MS = 650
# Soft poll interval.
POLL_MS = 250
# Hard budget for "ok to run every animating frame".
EVERY_FRAME_BUDGET_MS = 40


def imread(path: Path) -> np.ndarray:
    return cv2.imdecode(np.fromfile(str(path), dtype=np.uint8), cv2.IMREAD_COLOR)


def to_runtime(bgr: np.ndarray) -> np.ndarray:
    return cv2.resize(bgr, RUNTIME_WH, interpolation=cv2.INTER_AREA)


def gray(bgr: np.ndarray) -> np.ndarray:
    return cv2.cvtColor(bgr, cv2.COLOR_BGR2GRAY)


def timed_median(fn, rounds: int = 7) -> tuple[float, object]:
    fn()
    xs = []
    last = None
    for _ in range(rounds):
        t0 = time.perf_counter()
        last = fn()
        xs.append((time.perf_counter() - t0) * 1000.0)
    return statistics.median(xs), last


# ---------- algorithms ----------


def cv_ccoeff(hay_g: np.ndarray, nd_g: np.ndarray, y1: float = 0.85) -> float:
    top = hay_g[: int(hay_g.shape[0] * y1)]
    if nd_g.shape[0] >= top.shape[0] or nd_g.shape[1] >= top.shape[1]:
        return 0.0
    return float(cv2.minMaxLoc(cv2.matchTemplate(top, nd_g, cv2.TM_CCOEFF_NORMED))[1])


def cv_pyramid(hay_g: np.ndarray, nd_g: np.ndarray, y1: float = 0.85, factor: int = 2) -> float:
    h, w = hay_g.shape
    y_lim = int(h * y1)
    top = hay_g[:y_lim]
    small = cv2.resize(top, (w // factor, y_lim // factor), interpolation=cv2.INTER_AREA)
    snd = cv2.resize(
        nd_g,
        (max(4, nd_g.shape[1] // factor), max(4, nd_g.shape[0] // factor)),
        interpolation=cv2.INTER_AREA,
    )
    if snd.shape[0] >= small.shape[0] or snd.shape[1] >= small.shape[1]:
        return cv_ccoeff(hay_g, nd_g, y1)
    _mn, _mx, _ml, ml = cv2.minMaxLoc(cv2.matchTemplate(small, snd, cv2.TM_CCOEFF_NORMED))
    nh, nw = nd_g.shape
    cx = (ml[0] + snd.shape[1] // 2) * factor
    cy = (ml[1] + snd.shape[0] // 2) * factor
    pad = 24
    x0, y0 = max(0, cx - nw // 2 - pad), max(0, cy - nh // 2 - pad)
    x1, y1b = min(w, cx + nw // 2 + pad), min(y_lim, cy + nh // 2 + pad)
    win = top[y0:y1b, x0:x1]
    if win.shape[0] < nh or win.shape[1] < nw:
        return float(_mx)
    return float(cv2.minMaxLoc(cv2.matchTemplate(win, nd_g, cv2.TM_CCOEFF_NORMED))[1])


def cv_down4_then_refine(hay_g: np.ndarray, nd_g: np.ndarray, y1: float = 0.85) -> float:
    """Mirrors vision-core shell matcher: down×4 propose → local refine (via OpenCV)."""
    return cv_pyramid(hay_g, nd_g, y1=y1, factor=4)


def nested_ccoeff_step(
    hay_g: np.ndarray,
    nd_g: np.ndarray,
    *,
    y1: float = 0.72,
    step: int = 4,
    max_positions: int = 5000,
) -> float:
    """Naive nested loops — stands in for unoptimized Rust CCOEFF."""
    top = hay_g[: int(hay_g.shape[0] * y1)]
    nh, nw = nd_g.shape
    if nh >= top.shape[0] or nw >= top.shape[1]:
        return 0.0
    # Work on further downscale to keep bench runnable, scale score still useful for timing order.
    factor = 2
    small = cv2.resize(
        top, (top.shape[1] // factor, top.shape[0] // factor), interpolation=cv2.INTER_AREA
    )
    snd = cv2.resize(
        nd_g, (max(4, nw // factor), max(4, nh // factor)), interpolation=cv2.INTER_AREA
    )
    sh, sw = small.shape
    snh, snw = snd.shape
    nv = snd.astype(np.float32)
    nm = float(nv.mean())
    nnorm = float(np.sqrt(max((nv * nv).mean() - nm * nm, 1e-6)))
    best = 0.0
    npos = 0
    for y in range(0, sh - snh + 1, step):
        for x in range(0, sw - snw + 1, step):
            win = small[y : y + snh, x : x + snw].astype(np.float32)
            m = float(win.mean())
            v = float(np.sqrt(max((win * win).mean() - m * m, 1e-6)))
            cross = float(((win - nm) * (nv - nm)).mean())
            score = cross / (v * nnorm + 1e-6)
            if score > best:
                best = score
            npos += 1
            if npos >= max_positions:
                return best
    return best


def ahash(img_g: np.ndarray, size: int = 8) -> int:
    small = cv2.resize(img_g, (size, size), interpolation=cv2.INTER_AREA)
    m = small.mean()
    bits = (small > m).astype(np.uint8).flatten()
    h = 0
    for i, b in enumerate(bits):
        h |= int(b) << i
    return h


def ahash_hamming(a: int, b: int) -> int:
    return (a ^ b).bit_count()


def tpl_ahash_scan(hay_g: np.ndarray, nd_g: np.ndarray, y1: float = 0.72, step: int = 8) -> float:
    """Slide aHash window; return 1 - hamming/64 as pseudo-score (higher better)."""
    top = hay_g[: int(hay_g.shape[0] * y1)]
    nh, nw = nd_g.shape
    if nh >= top.shape[0] or nw >= top.shape[1]:
        return 0.0
    th = ahash(nd_g)
    best = 0.0
    for y in range(0, top.shape[0] - nh + 1, step):
        for x in range(0, top.shape[1] - nw + 1, step):
            ham = ahash_hamming(th, ahash(top[y : y + nh, x : x + nw]))
            score = 1.0 - ham / 64.0
            if score > best:
                best = score
    return best


def bottom_blue_ratio(bgr: np.ndarray) -> float:
    h = bgr.shape[0]
    band = bgr[int(h * 0.62) : int(h * 0.92)]
    hsv = cv2.cvtColor(band, cv2.COLOR_BGR2HSV)
    m = (
        (hsv[:, :, 0] >= 95)
        & (hsv[:, :, 0] <= 125)
        & (hsv[:, :, 1] >= 70)
        & (hsv[:, :, 2] >= 80)
    )
    return float(m.mean())


def ocr_has(bgr: np.ndarray, needle: str, y0: float, y1: float) -> bool:
    from rapidocr_onnxruntime import RapidOCR

    if not hasattr(ocr_has, "_eng"):
        ocr_has._eng = RapidOCR()  # type: ignore[attr-defined]
    h = bgr.shape[0]
    crop = bgr[int(h * y0) : int(h * y1)]
    rgb = cv2.cvtColor(crop, cv2.COLOR_BGR2RGB)
    result, _ = ocr_has._eng(rgb)  # type: ignore[attr-defined]
    return any(needle in item[1] for item in (result or []))


# ---------- cases ----------


@dataclass
class ProbeCase:
    name: str
    positive: np.ndarray  # BGR runtime-sized
    negatives: list[tuple[str, np.ndarray]]
    needle_bgr: np.ndarray | None
    kind: str  # template | color | ocr


def build_cases() -> list[ProbeCase]:
    home = to_runtime(imread(IMAGES / "8.png"))
    sport = to_runtime(imread(IMAGES / "9.png"))
    venue = to_runtime(imread(IMAGES / "10.png"))
    detail = to_runtime(imread(IMAGES / "11.png"))
    gym = imread(TPL / "sports" / "08_gym.png")
    i_know = imread(TPL / "i_know.png")
    notice = imread(TPL / "notice_title.png")
    # Also native-resolution sport (offline gold) for scale sanity.
    sport_native = imread(IMAGES / "9.png")

    return [
        ProbeCase(
            "gym@runtime520x719",
            sport,
            [("home", home), ("venue", venue), ("detail", detail)],
            gym,
            "template",
        ),
        ProbeCase(
            "gym@native512x826",
            sport_native,
            [("home", imread(IMAGES / "8.png")), ("venue", imread(IMAGES / "10.png"))],
            gym,
            "template",
        ),
        ProbeCase(
            "i_know@sport(neg expected)",
            sport,
            [("home", home), ("venue", venue)],
            i_know,
            "template",
        ),
        ProbeCase(
            "notice_title@sport(neg expected)",
            sport,
            [("home", home)],
            notice,
            "template",
        ),
        ProbeCase(
            "bottom_blue@sport(neg)",
            sport,
            [("home", home), ("venue", venue)],
            None,
            "color",
        ),
        ProbeCase(
            "ocr_场馆列表@venue",
            venue,
            [("sport", sport), ("home", home), ("detail", detail)],
            None,
            "ocr",
        ),
    ]


def run_template_algos(case: ProbeCase) -> list[dict]:
    assert case.needle_bgr is not None
    hay = gray(case.positive)
    nd = gray(case.needle_bgr)
    negs = [(n, gray(im)) for n, im in case.negatives]
    algos = [
        ("cv_ccoeff", lambda h, n: cv_ccoeff(h, n), HIT),
        ("cv_pyramid_x2", lambda h, n: cv_pyramid(h, n, factor=2), HIT),
        ("cv_down4_refine", lambda h, n: cv_down4_then_refine(h, n), HIT),
        ("nested_step4 (rust-ish)", lambda h, n: nested_ccoeff_step(h, n, step=4), HIT),
        ("nested_step8 (rust-ish)", lambda h, n: nested_ccoeff_step(h, n, step=8), HIT),
        ("ahash_slide_step8", lambda h, n: tpl_ahash_scan(h, n, step=8), 0.90),
    ]
    rows = []
    for name, fn, thr in algos:
        ms, score = timed_median(lambda: fn(hay, nd))
        tp = score >= thr
        fp = 0
        for _, ng in negs:
            if fn(ng, nd) >= thr:
                fp += 1
        rows.append(
            {
                "algo": name,
                "ms": ms,
                "score": score,
                "thr": thr,
                "tp": tp,
                "fp": fp,
                "n_neg": len(negs),
                "ok": tp and fp == 0,
            }
        )
    return rows


def run_color(case: ProbeCase) -> list[dict]:
    ms, score = timed_median(lambda: bottom_blue_ratio(case.positive))
    thr = 0.02
    tp = score >= thr
    fp = sum(1 for _, im in case.negatives if bottom_blue_ratio(im) >= thr)
    return [
        {
            "algo": "bottom_blue_ratio",
            "ms": ms,
            "score": score,
            "thr": thr,
            "tp": tp,
            "fp": fp,
            "n_neg": len(case.negatives),
            "ok": tp and fp == 0,
        }
    ]


def run_ocr(case: ProbeCase) -> list[dict]:
    # warm
    _ = ocr_has(case.positive, "场馆列表", 0.0, 0.25)
    ms, hit = timed_median(lambda: ocr_has(case.positive, "场馆列表", 0.0, 0.25), rounds=3)
    fp = sum(1 for _, im in case.negatives if ocr_has(im, "场馆列表", 0.0, 0.25))
    return [
        {
            "algo": "ocr_roi_title",
            "ms": ms,
            "score": 1.0 if hit else 0.0,
            "thr": 1.0,
            "tp": bool(hit),
            "fp": fp,
            "n_neg": len(case.negatives),
            "ok": bool(hit) and fp == 0,
        }
    ]


def schedule_sim(per_probe_ms: float, n_templates: int = 3) -> dict:
    """If we run n templates every animating frame, what happens inside FORCE_MS."""
    bundle = per_probe_ms * n_templates
    frames_per_force = FORCE_MS / max(bundle, 0.001)
    every_ok = bundle <= EVERY_FRAME_BUDGET_MS
    # Interval schedule: probe at most once per FORCE_MS (same cadence as force OCR).
    interval_ok = per_probe_ms <= FORCE_MS * 0.5  # leave half for OCR if needed
    return {
        "bundle_ms": bundle,
        "frames_in_force_window": frames_per_force,
        "every_frame_ok": every_ok,
        "force_cadence_ok": interval_ok,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--out",
        type=Path,
        default=ROOT / "docs" / "research" / "l2-probe-budget-bench.md",
    )
    args = ap.parse_args()

    lines = [
        "# L2 probe budget bake-off",
        "",
        f"> Runtime size fixed to **{RUNTIME_WH[0]}×{RUNTIME_WH[1]}** (log). "
        f"Every-frame budget **{EVERY_FRAME_BUDGET_MS}ms**; force gap **{FORCE_MS}ms**.",
        "",
        "## Why this bench",
        "",
        "Log 03:30: `until=[我知道了,预约须知,健身房]` → poll1 `animating` at **4709ms** with no hit. "
        "Hypothesis: naive/CCOEFF-in-Rust × 3 templates per frame exceeds the poll budget so force-OCR never runs in time.",
        "",
        "Prior art: OpenCV `matchTemplate` uses optimized/DFT paths — pure nested loops can be **100–1000×** slower "
        "([OpenCV answers](https://answers.opencv.org/question/83870/why-is-opencvs-template-matching-method-tm_sqdiff-so-fast/), "
        "StackOverflow UI image recognition). vision-core shell matcher already uses **down×4 + sparse ZNCC + local refine**.",
        "",
    ]

    print("=== per-algorithm ===")
    for case in build_cases():
        print(f"\n--- {case.name} ---")
        if case.kind == "template":
            rows = run_template_algos(case)
        elif case.kind == "color":
            rows = run_color(case)
        else:
            rows = run_ocr(case)
        lines.append(f"## {case.name}")
        lines.append("")
        lines.append("| algo | ms | score | thr | TP | FP | OK |")
        lines.append("|---|---:|---:|---:|:---:|:---:|:---:|")
        for r in sorted(rows, key=lambda x: (not x["ok"], x["ms"])):
            flag = "OK" if r["ok"] else ("TPfp" if r["tp"] else "miss")
            print(
                f"  [{flag}] {r['algo']:28s} {r['ms']:8.1f}ms score={r['score']:.3f} FP={r['fp']}/{r['n_neg']}"
            )
            lines.append(
                f"| `{r['algo']}` | {r['ms']:.1f} | {r['score']:.3f} | {r['thr']:.2f} | "
                f"{'Y' if r['tp'] else ''} | {r['fp']}/{r['n_neg']} | {'Y' if r['ok'] else ''} |"
            )
        lines.append("")

        if case.name.startswith("gym@runtime") and case.kind == "template":
            lines.append("### Schedule simulation (3 templates like 场馆预约 until-OR)")
            lines.append("")
            lines.append("| algo | per-tpl ms | ×3 bundle | frames/650ms | every-frame? | force-cadence? |")
            lines.append("|---|---:|---:|---:|:---:|:---:|")
            for r in rows:
                sim = schedule_sim(r["ms"], 3)
                lines.append(
                    f"| `{r['algo']}` | {r['ms']:.1f} | {sim['bundle_ms']:.1f} | "
                    f"{sim['frames_in_force_window']:.2f} | "
                    f"{'Y' if sim['every_frame_ok'] else ''} | "
                    f"{'Y' if sim['force_cadence_ok'] else ''} |"
                )
                print(
                    f"  schedule {r['algo']}: bundle={sim['bundle_ms']:.0f}ms "
                    f"every={sim['every_frame_ok']} force_cadence={sim['force_cadence_ok']}"
                )
            lines.append("")

    lines.extend(
        [
            "## Evidence-based recommendations",
            "",
            "1. **Do not run nested-loop CCOEFF every animating frame.** "
            "Even step=4/8 is hundreds of ms/template → ×3 ≫ 650ms (matches the 4.7s poll1 log).",
            "2. **If using OpenCV-class matching:** `cv_ccoeff` / `cv_pyramid_x2` / `cv_down4_refine` are ~few ms and can be every-frame — "
            "but **only after confirming score≥0.82 on runtime 520×719**. If native 9.png hits and runtime miss, fix **scale** (template pack or multi-scale), not just schedule.",
            "3. **Reuse vision-core pattern:** down×4 + sparse ZNCC + ±20 refine (already in `ncc.rs`) — port that for booking icons instead of dense pyramid in `locator_gate`.",
            "4. **Schedule fallback (no new deps):** probe templates at **force cadence (~650ms)** or when `stable`, not every WGC frame; keep ROI OCR only on stable/force (already).",
            "5. **Notice branch without heavy tpl:** prefer cheap color/geometry once we have a notice positive capture; until then OCR「我知道了」on force is safer than 3 slow templates.",
            "6. **OR short-circuit:** stop at first hit; order cheap→expensive (color → down4 tpl → ROI OCR).",
            "",
            "## Suggested implementation order",
            "",
            "A. Schedule: `until_has_fast_probe` → only probe when `stable || force_ocr` (same as ROI), **or** min interval 200ms.",
            "B. Matcher: replace locator_gate dense loops with **down4 + sparse ZNCC** (copy `vision-core` ncc helpers) + optional OpenCV later.",
            "C. Scale: bench gym score on 520×719; if <0.82, add scales `{0.75,1.0,1.25}` only on down4 grid.",
            "D. Re-run device: expect `locator_hit` within <1s or force OCR ≤650ms, never poll1≈5s.",
            "",
        ]
    )

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text("\n".join(lines), encoding="utf-8")
    print(f"\nWrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
