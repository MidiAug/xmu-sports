/**
 * 与 src/lib/collapse-step-cycles.ts 算法对齐的金标检查（键已是人话行）。
 * Run: node scripts/check_collapse_step_cycles.mjs
 */
const PMAX = 4;
const MIN_K = 3;

function collapse(keys) {
  const out = [];
  let i = 0;
  const n = keys.length;
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
      out.push({ lines: keys.slice(i, i + bestP), k: bestK });
      i += bestK * bestP;
    } else {
      out.push({ lines: [keys[i]], k: 1 });
      i += 1;
    }
  }
  return out;
}

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

function eq(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

// AC-UX-HIST-06: AB ×3
{
  const keys = [];
  for (let i = 0; i < 6; i++) keys.push(i % 2 === 0 ? "A" : "B");
  const r = collapse(keys);
  assert(eq(r, [{ lines: ["A", "B"], k: 3 }]), "AB×3 failed: " + JSON.stringify(r));
}

// AC-UX-HIST-07: AB ×2 不折
{
  const keys = ["A", "B", "A", "B"];
  const r = collapse(keys);
  assert(
    eq(r, [
      { lines: ["A"], k: 1 },
      { lines: ["B"], k: 1 },
      { lines: ["A"], k: 1 },
      { lines: ["B"], k: 1 },
    ]),
    "AB×2 should not collapse: " + JSON.stringify(r),
  );
}

// AC-UX-HIST-08: AAA ×3
{
  const r = collapse(["A", "A", "A"]);
  assert(eq(r, [{ lines: ["A"], k: 3 }]), "A×3 failed: " + JSON.stringify(r));
}

// AC-UX-HIST-09: AB×3 + 残段 A
{
  const keys = ["A", "B", "A", "B", "A", "B", "A"];
  const r = collapse(keys);
  assert(
    eq(r, [
      { lines: ["A", "B"], k: 3 },
      { lines: ["A"], k: 1 },
    ]),
    "residual failed: " + JSON.stringify(r),
  );
}

// 平手取小 p：AAAAAA → A×6 而非 AA×3
{
  const r = collapse(["A", "A", "A", "A", "A", "A"]);
  assert(eq(r, [{ lines: ["A"], k: 6 }]), "tie-break small p failed: " + JSON.stringify(r));
}

console.log("[PASS] collapse_step_cycles checks");
