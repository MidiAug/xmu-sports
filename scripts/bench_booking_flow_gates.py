#!/usr/bin/env python3
"""Flow-first locator bake-off: what each booking click should wait for.

First-principles: PC 微信小程序没有 DOM。点后就绪 = 「下一步可行动的可见锚」出现，
不是「整页 taxonomy 完美」也不是「像素完全静止」。

This script:
  1. Enumerates each product transition and its correct until-oracle(s).
  2. Runs cheap→expensive probes on positive + negative screens.
  3. Scores speed (median ms) and quality (hit on positive, false-positive on negatives).

Usage:
  python scripts/bench_booking_flow_gates.py
  python scripts/bench_booking_flow_gates.py --rounds 5 --out docs/research/booking-flow-gate-bench.md
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

HIT_TPL = 0.82


def imread(path: Path) -> np.ndarray:
    data = np.fromfile(str(path), dtype=np.uint8)
    img = cv2.imdecode(data, cv2.IMREAD_COLOR)
    if img is None:
        raise FileNotFoundError(path)
    return img


def timed(fn, rounds: int) -> tuple[float, object]:
    fn()
    xs: list[float] = []
    last = None
    for _ in range(rounds):
        t0 = time.perf_counter()
        last = fn()
        xs.append((time.perf_counter() - t0) * 1000.0)
    return statistics.median(xs), last


# ---------- probes ----------


def mad(a: np.ndarray, b: np.ndarray) -> float:
    if a.shape != b.shape:
        b = cv2.resize(b, (a.shape[1], a.shape[0]))
    return float(np.abs(a.astype(np.float32) - b.astype(np.float32)).mean())


def ahash64(img: np.ndarray) -> int:
    g = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    small = cv2.resize(g, (8, 8), interpolation=cv2.INTER_AREA)
    mean = small.mean()
    bits = (small > mean).astype(np.uint8).flatten()
    h = 0
    for i, b in enumerate(bits):
        h |= int(b) << i
    return h


def hamming(a: int, b: int) -> int:
    return (a ^ b).bit_count()


def tpl_ccoeff_roi(hay: np.ndarray, needle: np.ndarray, y1: float = 0.85) -> float:
    crop = hay[: int(hay.shape[0] * y1)]
    hg = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    ng = cv2.cvtColor(needle, cv2.COLOR_BGR2GRAY)
    if ng.shape[0] >= hg.shape[0] or ng.shape[1] >= hg.shape[1]:
        return 0.0
    res = cv2.matchTemplate(hg, ng, cv2.TM_CCOEFF_NORMED)
    return float(cv2.minMaxLoc(res)[1])


def tpl_pyramid(hay: np.ndarray, needle: np.ndarray, y1: float = 0.85) -> float:
    h, w = hay.shape[:2]
    y_lim = int(h * y1)
    crop = hay[:y_lim]
    hg = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    ng = cv2.cvtColor(needle, cv2.COLOR_BGR2GRAY)
    factor = 2
    small_h = cv2.resize(hg, (w // factor, y_lim // factor), interpolation=cv2.INTER_AREA)
    small_n = cv2.resize(
        ng,
        (max(4, ng.shape[1] // factor), max(4, ng.shape[0] // factor)),
        interpolation=cv2.INTER_AREA,
    )
    if small_n.shape[0] >= small_h.shape[0] or small_n.shape[1] >= small_h.shape[1]:
        return 0.0
    res = cv2.matchTemplate(small_h, small_n, cv2.TM_CCOEFF_NORMED)
    _mn, mx, _ml, ml = cv2.minMaxLoc(res)
    cx = (ml[0] + small_n.shape[1] // 2) * factor
    cy = (ml[1] + small_n.shape[0] // 2) * factor
    pad = 24
    nh, nw = ng.shape[:2]
    x0 = max(0, cx - nw // 2 - pad)
    y0 = max(0, cy - nh // 2 - pad)
    x1 = min(w, cx + nw // 2 + pad)
    y1b = min(y_lim, cy + nh // 2 + pad)
    window = hg[y0:y1b, x0:x1]
    if ng.shape[0] > window.shape[0] or ng.shape[1] > window.shape[1]:
        return float(mx)
    return float(cv2.minMaxLoc(cv2.matchTemplate(window, ng, cv2.TM_CCOEFF_NORMED))[1])


def tpl_multiscale(hay: np.ndarray, needle: np.ndarray, scales: list[float]) -> float:
    crop = hay[: int(hay.shape[0] * 0.85)]
    hg = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    ng0 = cv2.cvtColor(needle, cv2.COLOR_BGR2GRAY)
    best = 0.0
    for s in scales:
        nw, nh = max(8, int(ng0.shape[1] * s)), max(8, int(ng0.shape[0] * s))
        if nh >= hg.shape[0] or nw >= hg.shape[1]:
            continue
        nd = cv2.resize(ng0, (nw, nh), interpolation=cv2.INTER_AREA if s < 1 else cv2.INTER_LINEAR)
        best = max(best, float(cv2.minMaxLoc(cv2.matchTemplate(hg, nd, cv2.TM_CCOEFF_NORMED))[1]))
    return best


def bottom_blue_ratio(img: np.ndarray) -> float:
    """Notice CTA is a blue strip near the bottom of the white card."""
    h = img.shape[0]
    band = img[int(h * 0.62) : int(h * 0.92)]
    hsv = cv2.cvtColor(band, cv2.COLOR_BGR2HSV)
    mask = (
        (hsv[:, :, 0] >= 95)
        & (hsv[:, :, 0] <= 125)
        & (hsv[:, :, 1] >= 70)
        & (hsv[:, :, 2] >= 80)
    )
    return float(mask.mean())


def mint_ratio(img: np.ndarray) -> float:
    hsv = cv2.cvtColor(img, cv2.COLOR_BGR2HSV)
    pale = (
        (hsv[:, :, 0] >= 68)
        & (hsv[:, :, 0] <= 98)
        & (hsv[:, :, 1] >= 10)
        & (hsv[:, :, 1] <= 95)
        & (hsv[:, :, 2] >= 200)
    )
    strong = (
        (hsv[:, :, 0] >= 35)
        & (hsv[:, :, 0] <= 100)
        & (hsv[:, :, 1] >= 40)
        & (hsv[:, :, 2] >= 140)
    )
    return float(np.logical_or(pale, strong).mean())


def white_center_ratio(img: np.ndarray) -> float:
    """Notice white sheet occupies center; home/sport are busier."""
    h, w = img.shape[:2]
    roi = img[int(h * 0.18) : int(h * 0.78), int(w * 0.08) : int(w * 0.92)]
    return float(((roi[:, :, 0] > 245) & (roi[:, :, 1] > 245) & (roi[:, :, 2] > 245)).mean())


_OCR = None


def ocr():
    global _OCR
    if _OCR is None:
        from rapidocr_onnxruntime import RapidOCR

        _OCR = RapidOCR()
    return _OCR


def ocr_has(img: np.ndarray, needle: str, y0: float = 0.0, y1: float = 1.0) -> bool:
    h = img.shape[0]
    crop = img[int(h * y0) : int(h * y1)]
    rgb = cv2.cvtColor(crop, cv2.COLOR_BGR2RGB)
    result, _ = ocr()(rgb)
    texts = [i[1] for i in (result or [])]
    return any(needle in t for t in texts)


# ---------- transitions ----------


@dataclass
class ProbeResult:
    name: str
    ms: float
    score: float
    hit: bool


@dataclass
class Transition:
    name: str
    clicked: str
    # Correct product until (may be OR)
    until_desc: str
    positive: Path  # screen where until SHOULD fire
    negatives: list[Path]  # screens where until must NOT fire
    probes: list  # callables(img) -> (score, hit)


def build_transitions() -> list[Transition]:
    img = {n: IMAGES / f"{n}.png" for n in ["8", "9", "10", "11", "12-1", "13"]}
    gym = imread(TPL / "sports" / "08_gym.png")
    venue_btn = imread(TPL / "venue_booking.png")
    i_know = imread(TPL / "i_know.png")
    notice = imread(TPL / "notice_title.png")

    def probes_notice():
        return [
            ("tpl_i_know_pyramid", lambda im: (s := tpl_pyramid(im, i_know), s >= HIT_TPL)),
            ("tpl_i_know_ccoeff", lambda im: (s := tpl_ccoeff_roi(im, i_know), s >= HIT_TPL)),
            ("tpl_notice_title", lambda im: (s := tpl_ccoeff_roi(im, notice), s >= HIT_TPL)),
            ("color_bottom_blue>0.02", lambda im: (s := bottom_blue_ratio(im), s >= 0.02)),
            ("white_center>0.55", lambda im: (s := white_center_ratio(im), s >= 0.55)),
            ("ocr_我知道了_roi", lambda im: ((h := ocr_has(im, "我知道了", 0.55, 0.95), 1.0 if h else 0.0)[::-1])),
            ("ocr_预约须知_roi", lambda im: ((h := ocr_has(im, "预约须知", 0.1, 0.5), 1.0 if h else 0.0)[::-1])),
        ]

    def probes_sport():
        return [
            ("tpl_gym_pyramid", lambda im: (s := tpl_pyramid(im, gym, 0.72), s >= HIT_TPL)),
            ("tpl_gym_ccoeff_roi", lambda im: (s := tpl_ccoeff_roi(im, gym, 0.72), s >= HIT_TPL)),
            ("tpl_gym_ms_coarse", lambda im: (s := tpl_multiscale(im, gym, [0.55, 0.7, 0.85, 1.0, 1.2, 1.4]), s >= HIT_TPL)),
            ("ocr_健身房_roi055", lambda im: ((h := ocr_has(im, "健身房", 0.05, 0.55), 1.0 if h else 0.0)[::-1])),
            ("ocr_full_健身房", lambda im: ((h := ocr_has(im, "健身房"), 1.0 if h else 0.0)[::-1])),
        ]

    def probes_venue_row():
        return [
            ("ocr_翔安_roi055", lambda im: ((h := ocr_has(im, "翔安校区健身房", 0.05, 0.55), 1.0 if h else 0.0)[::-1])),
            ("ocr_翔安_full", lambda im: ((h := ocr_has(im, "翔安校区健身房"), 1.0 if h else 0.0)[::-1])),
            ("ocr_场馆列表_title", lambda im: ((h := ocr_has(im, "场馆列表", 0.0, 0.25), 1.0 if h else 0.0)[::-1])),
        ]

    def probes_home_entry():
        return [
            ("tpl_venue_booking_pyramid", lambda im: (s := tpl_pyramid(im, venue_btn), s >= HIT_TPL)),
            ("tpl_venue_booking_ccoeff", lambda im: (s := tpl_ccoeff_roi(im, venue_btn), s >= HIT_TPL)),
            ("ocr_场馆预约_y35_70", lambda im: ((h := ocr_has(im, "场馆预约", 0.35, 0.70), 1.0 if h else 0.0)[::-1])),
        ]

    def probes_slot_mint():
        return [
            ("mint_ratio>0.03", lambda im: (s := mint_ratio(im), s >= 0.03)),
            ("mint_ratio>0.02", lambda im: (s := mint_ratio(im), s >= 0.02)),
        ]

    # NOTE: we lack a dedicated notice screenshot in images/. Use synthetic OR
    # document that notice probes are evaluated on 9 (neg) and home (neg) only
    # until a notice capture exists. Positive for notice = unavailable → skip TP.
    return [
        Transition(
            name="T0 identity: still on home (before click takes effect)",
            clicked="(pre)",
            until_desc="场馆预约 still visible (unchanged / wrong to advance)",
            positive=img["8"],
            negatives=[img["9"], img["10"]],
            probes=probes_home_entry(),
        ),
        Transition(
            name="T1 after「场馆预约」: notice OR sport (branch)",
            clicked="场馆预约",
            until_desc="OR(我知道了/须知, 健身房图标) — current code only waits 健身房 → wrong on notice",
            positive=img["9"],  # sport branch positive; notice branch needs capture
            negatives=[img["8"], img["10"]],
            probes=probes_sport() + probes_notice(),
        ),
        Transition(
            name="T2 after「我知道了」: sport grid",
            clicked="我知道了",
            until_desc="健身房图标 (template)",
            positive=img["9"],
            negatives=[img["8"], img["10"], img["11"]],
            probes=probes_sport(),
        ),
        Transition(
            name="T3 after「健身房」: venue list row",
            clicked="健身房",
            until_desc="翔安校区健身房 (ROI OCR)",
            positive=img["10"],
            negatives=[img["8"], img["9"], img["11"]],
            probes=probes_venue_row(),
        ),
        Transition(
            name="T4 after venue: detail date cards (pred 预定/暂无 — geometry later)",
            clicked="翔安校区健身房",
            until_desc="场馆详情正文（本轮用 OCR「预定|暂无|今天」作弱探针）",
            positive=img["11"],
            negatives=[img["9"], img["10"]],
            probes=[
                ("ocr_预定_or_暂无", lambda im: ((h := (ocr_has(im, "预定") or ocr_has(im, "暂无")), 1.0 if h else 0.0)[::-1])),
                ("ocr_今天", lambda im: ((h := ocr_has(im, "今天"), 1.0 if h else 0.0)[::-1])),
            ],
        ),
        Transition(
            name="T5 select-slot: mint available cells",
            clicked="预定",
            until_desc="mint cells present (color, not OCR)",
            positive=img["12-1"],
            negatives=[img["11"], img["10"], img["13"]],
            probes=probes_slot_mint(),
        ),
    ]


def run_transition(tr: Transition, rounds: int) -> list[dict]:
    pos = imread(tr.positive)
    negs = [imread(p) for p in tr.negatives]
    rows = []
    print(f"\n=== {tr.name} ===")
    print(f"  clicked={tr.clicked}  until={tr.until_desc}")
    print(f"  + {tr.positive.name}   − {[p.name for p in tr.negatives]}")

    for name, fn in tr.probes:
        ms, (score, hit_pos) = timed(lambda: fn(pos), rounds)
        fp = 0
        fp_scores = []
        for nimg in negs:
            s, h = fn(nimg)
            fp_scores.append(s)
            if h:
                fp += 1
        # quality: TP on positive, FP count on negatives
        ok = hit_pos and fp == 0
        flag = "OK " if ok else ("TPfp" if hit_pos else "MISS")
        print(
            f"  [{flag}] {name:28s} {ms:7.1f}ms  pos={score:.3f} hit={hit_pos}  FP={fp}/{len(negs)}"
        )
        rows.append(
            {
                "probe": name,
                "ms": ms,
                "pos_score": score,
                "tp": hit_pos,
                "fp": fp,
                "n_neg": len(negs),
                "ok": ok,
            }
        )
    return rows


def pairwise_change_signals(rounds: int) -> str:
    """Layer-1: how fast/ reliably can we detect screen change 8→9→10."""
    a, b, c = imread(IMAGES / "8.png"), imread(IMAGES / "9.png"), imread(IMAGES / "10.png")
    lines = ["## Layer-1 change signals (8→9→10)", ""]
    pairs = [("8→9", a, b), ("9→10", b, c), ("8→8", a, a)]
    for label, x, y in pairs:
        ms_mad, mad_v = timed(lambda: mad(x, y), rounds)
        ms_ah, ah_v = timed(lambda: hamming(ahash64(x), ahash64(y)), rounds)
        lines.append(
            f"- {label}: MAD={mad_v:.2f} ({ms_mad:.2f}ms), aHash_hamming={ah_v} ({ms_ah:.2f}ms)"
        )
    lines.append("")
    lines.append(
        "Interpretation: MAD/aHash only answer「变了没」. They do **not** answer「下一步锚点在不在」。"
        " Use as skip gate (unchanged→don't probe), never as done-oracle."
    )
    lines.append("")
    return "\n".join(lines)


def to_markdown(all_rows: list[tuple[Transition, list[dict]]], layer1: str) -> str:
    lines = [
        "# Booking-flow gate bake-off (first principles)",
        "",
        "> `python scripts/bench_booking_flow_gates.py`",
        "",
        "## 第一性原理",
        "",
        "PC 微信小程序路径上，自动化能依赖的只有像素。产品真源是"
        "[wechat-booking-flow](wechat-booking-flow.md)：**点哪里 → 下一步该看见什么**，"
        "不是分类器页表。",
        "",
        "点后就绪应拆三层（对齐 Playwright explicit wait / Airtest `wait(Template)` / WEFix desired-state）：",
        "",
        "1. **L1 变化门** — 相对点前是否变屏（fingerprint / MAD / aHash）。未变 → 不花 OCR。",
        "2. **L2 目标锚** — 下一步可点控件是否出现（模板 / 色块 / ROI 文案）。命中 → 才允许全量 observe。",
        "3. **L3 全量观察** — RapidOCR + classify + decide（~0.5–3s）。只为规划下一击，不作轮询探针。",
        "",
        "业界共识：等 **desired state**，不要等 networkidle / 整页静止"
        "（[Playwright](https://playwright.dev/docs/navigations)、[WEFix](https://ar5iv.labs.arxiv.org/html/2402.09745)、"
        "[Airtest](https://airtest.doc.io.netease.com/)）。",
        "",
        layer1,
        "## Per-transition results",
        "",
    ]
    for tr, rows in all_rows:
        lines.append(f"### {tr.name}")
        lines.append("")
        lines.append(f"- clicked: `{tr.clicked}`")
        lines.append(f"- until: {tr.until_desc}")
        lines.append(f"- positive: `{tr.positive.name}` · negatives: `{[p.name for p in tr.negatives]}`")
        lines.append("")
        lines.append("| probe | ms | TP | FP | score | OK |")
        lines.append("|---|---:|:---:|:---:|---:|:---:|")
        for r in sorted(rows, key=lambda x: (not x["ok"], x["ms"])):
            lines.append(
                f"| `{r['probe']}` | {r['ms']:.1f} | {'Y' if r['tp'] else ''} | "
                f"{r['fp']}/{r['n_neg']} | {r['pos_score']:.3f} | {'Y' if r['ok'] else ''} |"
            )
        winners = [r for r in rows if r["ok"]]
        if winners:
            best = sorted(winners, key=lambda x: x["ms"])[0]
            lines.append("")
            lines.append(
                f"**推荐:** `{best['probe']}` — {best['ms']:.1f}ms, TP, FP=0"
            )
        else:
            lines.append("")
            lines.append("**本轮无同时满足 TP∧FP=0 的探针**（缺须知截图或阈值需调）。")
        lines.append("")

    lines.extend(
        [
            "## 对现行实现的含义",
            "",
            "1. **T1「场馆预约」** 的 `until=健身房` 不完整：须知弹层挡住九宫格时，健身房模板不应作为唯一完成条件；"
            "应 `OR(须知/我知道了, 健身房)`，先点掉须知再等运动图标。",
            "2. **运动图标** 用 OpenCV 稠密/`pyramid` 灰图匹配（~1–6ms，分数≥0.94），"
            "不要用粗步进自写 NCC（离线仿真会掉到 ~0.56 → 真机 miss）。",
            "3. **场馆行** 用 ROI OCR `y∈[0.05,0.55]`，不要全页 OCR 作门禁。",
            "4. **场次可约** 用 mint 色占比/格色，不要等 OCR 读完 `0/1`。",
            "5. L1 指纹只负责「跳过未变帧」；`stable` 像素静止不是成功条件——目标锚出现即可全量 observe。",
            "",
            "## 缺口",
            "",
            "- `images/` 缺「预约须知」弹层真机图；T1 的 notice 正例无法在本仓库离线验证，需补采一张。",
            "- 详情页日期卡「预定/暂无」应优先几何+色，本脚本 OCR 只作弱对照。",
            "",
        ]
    )
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument(
        "--out",
        type=Path,
        default=ROOT / "docs" / "research" / "booking-flow-gate-bench.md",
    )
    args = ap.parse_args()

    # warm OCR
    ocr()
    _ = ocr_has(imread(IMAGES / "9.png"), "暖")

    layer1 = pairwise_change_signals(args.rounds)
    print(layer1)

    all_rows = []
    for tr in build_transitions():
        if not tr.positive.exists():
            print("skip", tr.name)
            continue
        all_rows.append((tr, run_transition(tr, args.rounds)))

    md = to_markdown(all_rows, layer1)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(md, encoding="utf-8")
    print(f"\nWrote {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
