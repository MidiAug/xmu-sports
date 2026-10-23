import { formatStepLine } from "./format-human";

export type StepCycleLine = { line: string; ts: string };

export type StepCycleBlock = {
  /** pattern 原文行（长度为周期 p） */
  lines: StepCycleLine[];
  /** 完整周期次数；单行未折叠为 1 */
  k: number;
  tsFirst: string;
  tsLast: string;
};

const PMAX = 4;
const MIN_K = 3;

type StepEvent = {
  action?: string | null;
  page_id?: string | null;
  ts: string;
};

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

function parseMaybeIso(iso: string): Date | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t);
}

/** 展开步骤用：仅时分秒，不带今天/昨天 */
export function formatStepClock(ts: string): string {
  const d = parseMaybeIso(ts);
  if (!d) return ts;
  const china = new Date(d.getTime()+8*3600_000);
  return `${pad2(china.getUTCHours())}:${pad2(china.getUTCMinutes())}:${pad2(china.getUTCSeconds())}`;
}

/** 同日/跨日一律 `HH:MM:SS~HH:MM:SS`；首末相同则单点时钟 */
export function formatStepTimeRange(tsFirst: string, tsLast: string): string {
  if (tsFirst === tsLast) return formatStepClock(tsFirst);
  return `${formatStepClock(tsFirst)}~${formatStepClock(tsLast)}`;
}

/**
 * 贪心周期折叠：完整重复 k≥3 才折；p=1…Pmax；平手取更小 p。
 * @see docs/superpowers/specs/2026-09-23-history-step-cycle-collapse-design.md
 */
export function collapseStepCycles(events: StepEvent[]): StepCycleBlock[] {
  const keys = events.map((e) => formatStepLine(e));
  const out: StepCycleBlock[] = [];
  let i = 0;
  const n = events.length;

  while (i < n) {
    const remain = n - i;
    let bestP = 0;
    let bestK = 0;
    let bestCover = 0;

    const pMax = Math.min(PMAX, Math.floor(remain / MIN_K));
    for (let p = 1; p <= pMax; p++) {
      let k = 0;
      while (i + (k + 1) * p <= n) {
        let match = true;
        for (let j = 0; j < p; j++) {
          if (keys[i + k * p + j] !== keys[i + j]) {
            match = false;
            break;
          }
        }
        if (!match) break;
        k += 1;
      }
      if (k >= MIN_K) {
        const cover = k * p;
        if (cover > bestCover || (cover === bestCover && (bestP === 0 || p < bestP))) {
          bestCover = cover;
          bestP = p;
          bestK = k;
        }
      }
    }

    if (bestK >= MIN_K) {
      const lines: StepCycleLine[] = [];
      for (let j = 0; j < bestP; j++) {
        lines.push({ line: keys[i + j]!, ts: events[i + j]!.ts });
      }
      const lastIdx = i + bestK * bestP - 1;
      out.push({
        lines,
        k: bestK,
        tsFirst: events[i]!.ts,
        tsLast: events[lastIdx]!.ts,
      });
      i += bestK * bestP;
    } else {
      out.push({
        lines: [{ line: keys[i]!, ts: events[i]!.ts }],
        k: 1,
        tsFirst: events[i]!.ts,
        tsLast: events[i]!.ts,
      });
      i += 1;
    }
  }

  return out;
}
