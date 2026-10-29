import { invoke } from "@tauri-apps/api/core";
import type { RunEvent, SelectOption } from "../types";
import {
  formatStepClock,
} from "../lib/collapse-step-cycles";
import { historyStages, historyTargetParts } from "../lib/history-steps";
import { formatEventTimeHuman, pageLabelZh } from "../lib/format-human";
import { groupEventsIntoRuns, type RunSummary } from "../lib/run-groups";
import type { OutcomeKind } from "../lib/run-outcome";
import { askConfirm } from "../ui/confirm-ask";
import { mountShellSelect } from "../ui/select";
import { toastError, toastSuccess } from "../ui/toast";
import { mountPage } from "../lib/page-lifecycle";
import { localDateISO } from "../lib/time";

type KindFilter = "all" | "success" | "error" | "other";
type RangeFilter = "all" | "today" | "yesterday" | "week";

type HistoryFilters = {
  kind: KindFilter;
  range: RangeFilter;
};

type HistoryListState = {
  allRuns: RunSummary[];
  visibleCount?: number;
};

let cachedRuns: RunSummary[] | undefined;
let pendingHistory: Promise<RunSummary[]> | undefined;
let historyEpoch = 0;
export function preloadHistory(): Promise<RunSummary[]> { return loadHistory(); }
async function loadHistory(): Promise<RunSummary[]> {
  if (pendingHistory) return pendingHistory;
  const epoch = historyEpoch;
  const request = invoke<RunEvent[]>("list_recent_events", { limit: 0 }).then(events => {
    const runs = groupEventsIntoRuns(events);
    if (epoch !== historyEpoch) return cachedRuns ?? [];
    // Preserve the rendered view and open details if disk data did not change.
    if (!cachedRuns || JSON.stringify(cachedRuns) !== JSON.stringify(runs)) cachedRuns = runs;
    return cachedRuns;
  });
  pendingHistory = request;
  try { return await request; }
  finally { if (pendingHistory === request) pendingHistory = undefined; }
}

const KIND_OPTS: SelectOption[] = [
  { value: "all", label: "全部结果" },
  { value: "success", label: "成功" },
  { value: "error", label: "失败" },
  { value: "other", label: "其他" },
];

const RANGE_OPTS: SelectOption[] = [
  { value: "all", label: "全部时间" },
  { value: "today", label: "今天" },
  { value: "yesterday", label: "昨天" },
  { value: "week", label: "近7天" },
];

function escText(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function kindClass(kind: string): string {
  if (kind === "error") return "err";
  if (kind === "success") return "ok";
  return "info";
}

function startOfLocalDay(d: Date): number {
  return Date.parse(`${localDateISO(d)}T00:00:00+08:00`);
}

function matchesRange(ts: string, range: RangeFilter, now: Date): boolean {
  if (range === "all") return true;
  const t = Date.parse(ts);
  if (Number.isNaN(t)) return false;
  const dayDiff = Math.round(
    (startOfLocalDay(new Date(t)) - startOfLocalDay(now)) / 86_400_000,
  );
  if (range === "today") return dayDiff === 0;
  if (range === "yesterday") return dayDiff === -1;
  if (range === "week") return dayDiff <= 0 && dayDiff >= -6;
  return true;
}

function matchesKind(run: RunSummary, kind: KindFilter): boolean {
  if (kind === "all") return true;
  const k = (run.outcome?.kind ?? "info") as OutcomeKind;
  if (kind === "success") return k === "success";
  if (kind === "error") return k === "error";
  return k === "info" || k === "warn";
}

function filterRuns(runs: RunSummary[], f: HistoryFilters, now = new Date()): RunSummary[] {
  return runs.filter(
    (r) => matchesKind(r, f.kind) && matchesRange(r.endedAt, f.range, now),
  );
}

function cardHtml(run: RunSummary, index: number): string {
  const o = run.outcome;
  const title = o?.title ?? "运行记录";
  const kind = o?.kind ?? "info";
  const when = formatEventTimeHuman(run.endedAt);
  const page = o?.pageId ? pageLabelZh(o.pageId) : null;
  const metaSafe = page ? `${when}  ${page}` : when;

  return `
    <article class="run-card ${kindClass(kind)}" data-run-idx="${index}">
      <button type="button" class="run-card-head" aria-expanded="false" data-toggle-run="${index}">
        <span class="run-card-dot" aria-hidden="true"></span>
        <span class="run-card-text">
          <span class="run-card-title">${escText(title)}</span>
          <span class="run-card-meta">${escText(metaSafe)}</span>
        </span>
        <span class="run-card-chevron" aria-hidden="true"></span>
      </button>
      <div class="run-card-body" hidden></div>
    </article>`;
}

function cardBodyHtml(run: RunSummary): string {
  const detail = run.outcome?.detail ?? "";
  const blocks = historyStages(run.events);
  const target = historyTargetParts(run.events);
  const duration = Math.max(0,Date.parse(run.endedAt)-Date.parse(run.startedAt))/1000;
  const stepsHtml=blocks.length ? `<div class="run-stage-columns" aria-hidden="true"><span>过程</span><span>开始</span><span>耗时</span></div><ol class="run-stages" aria-label="过程">${blocks.map(block=>{
    const timing=formatStepClock(block.tsFirst);
    const elapsed=block.elapsedMs>0 ? `${(block.elapsedMs/1000).toFixed(1)} 秒` : "—";
    const detailRows=block.steps.flatMap(step=>step.lines.map(line=>`<li><time>${escText(formatStepClock(line.ts))}</time><span>${escText(line.line)}${step.k>1 ? `（连续 ${step.k} 次）` : ""}</span></li>`)).join("");
    const expanded=block.steps.length>1 || !["spawn","done","fail","cancelled"].includes(block.key) && block.steps[0].lines[0].line!==block.label;
    const row=`<span class="run-stage-chevron" aria-hidden="true"></span><span class="run-flow-label">${escText(block.label)}</span><time class="run-stage-start">${escText(timing)}</time><span class="run-stage-elapsed">${escText(elapsed)}</span>`;
    return `<li>${expanded ? `<details class="run-stage-details"><summary class="run-stage-row" aria-label="${escText(block.label)}，展开过程">${row}</summary><ol>${detailRows}</ol></details>` : `<div class="run-stage-row run-stage-static">${row}</div>`}</li>`;
  }).join("")}</ol>` : '<p class="run-flow-empty">没有过程步骤</p>';
  const extraDetail=detail && !blocks.some(block=>block.steps.some(step=>step.lines.some(line=>line.line.includes(detail))));

  return `        <div class="run-overview"><div>${target ? `<p class="run-target"><strong>${escText(target.sport)}</strong><span>${escText(target.venue)}</span></p><p class="run-target-meta"><span>${escText(target.date)}</span><span>${escText(target.time)}</span>${target.court && target.court!==target.venue ? `<span>${escText(target.court)}</span>` : ""}</p>` : ""}</div><div class="run-total"><span>总用时</span><strong>${duration.toFixed(1)}<small> 秒</small></strong></div></div>
        ${extraDetail ? `<p class="run-card-why">${escText(detail)}</p>` : ""}
        ${stepsHtml}`;
}

function emptyAllHtml(): string {
  return `
    <div class="empty history-empty">
      <p class="empty-title">还没有运行记录</p>
      <p class="empty-desc">单次预约、长期计划与登录检查的执行记录会显示在这里。</p>
    </div>`;
}

function emptyFilterHtml(): string {
  return `
    <div class="empty history-empty">
      <p class="empty-title">没有符合筛选的记录</p>
      <p class="empty-desc">换一组结果或时间条件再试。</p>
    </div>`;
}

function bindExpand(root: Element, runs: RunSummary[]) {
  root.querySelectorAll<HTMLButtonElement>("[data-toggle-run]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const card = btn.closest(".run-card");
      if (!card) return;
      const body = card.querySelector(".run-card-body");
      if (!body) return;
      const open = body.hasAttribute("hidden");
      if (open) {
        if (!body.hasAttribute("data-loaded")) {
          body.innerHTML = cardBodyHtml(runs[Number(btn.dataset.toggleRun)]);
          body.setAttribute("data-loaded", "");
        }
        body.removeAttribute("hidden");
      }
      else body.setAttribute("hidden", "");
      btn.setAttribute("aria-expanded", open ? "true" : "false");
      card.classList.toggle("open", open);
    });
  });
}

function paintList(panel: Element, state: HistoryListState, filters: HistoryFilters) {
  const filtered = filterRuns(state.allRuns, filters);
  const listHost = panel.querySelector("#history_list");
  if (!listHost) return;

  if (!state.allRuns.length) {
    listHost.innerHTML = emptyAllHtml();
  } else if (!filtered.length) {
    listHost.innerHTML = emptyFilterHtml();
  } else {
    const count = state.visibleCount ?? 30;
    listHost.innerHTML = filtered.slice(0, count).map((r, i) => cardHtml(r, i)).join("") +
      (filtered.length > count ? `<button type="button" class="btn ghost" id="history_more">加载更多（已显示 ${count} / ${filtered.length}）</button>` : "");
    bindExpand(listHost, filtered);
    listHost.querySelector<HTMLButtonElement>("#history_more")?.addEventListener("click", () => { state.visibleCount = count + 30; paintList(panel, state, filters); });
  }
}

function mountFilters(
  panel: Element,
  state: HistoryListState,
  filters: HistoryFilters,
) {
  const kindHost = panel.querySelector("#history_kind_host") as HTMLElement | null;
  const rangeHost = panel.querySelector("#history_range_host") as HTMLElement | null;
  const clearBtn = panel.querySelector(
    "#history_filter_clear",
  ) as HTMLButtonElement | null;
  if (!kindHost || !rangeHost) return;

  mountShellSelect(kindHost, KIND_OPTS, filters.kind, (v) => {
    state.visibleCount = 30;
    filters.kind = v as KindFilter;
    paintList(panel, state, filters);
  });
  mountShellSelect(rangeHost, RANGE_OPTS, filters.range, (v) => {
    state.visibleCount = 30;
    filters.range = v as RangeFilter;
    paintList(panel, state, filters);
  });

  clearBtn?.addEventListener("click", () => {
    void askConfirm({
      title: "清空运行记录",
      lead: "将删除本机保存的全部运行摘要，此操作不可恢复。",
      confirmLabel: "清空",
      danger: true,
    }).then(async (ok) => {
      if (!ok) return;
      const cleared = state.allRuns.length;
      try {
        await invoke<string>("clear_run_history");
        historyEpoch++;
        cachedRuns = [];
        pendingHistory = undefined;
        state.allRuns = [];
        paintList(panel, state, filters);
        toastSuccess(
          "已清空",
          cleared > 0 ? `已删除 ${cleared} 次运行摘要` : "没有可清空的记录",
        );
      } catch (e) {
        toastError("清空失败", String(e));
      }
    });
  });
}

export async function renderHistory(panel: Element) {
  const scope = mountPage(panel);
  const state: HistoryListState = { allRuns: cachedRuns ?? [] };
  const filters: HistoryFilters = { kind: "all", range: "all" };
  const epoch = historyEpoch;
  try {
    panel.innerHTML = `
      <div class="page-stack history-page">
        <div class="page-heading"><h2>活动记录</h2></div>
        <div class="history-summary-group">
          <div class="history-filter-selects">
            <div id="history_kind_host"></div>
            <div id="history_range_host"></div>
            <button
              type="button"
              class="btn ghost history-filter-clear"
              id="history_filter_clear"
            >清空记录</button>
          </div>
          <div class="run-card-list" id="history_list"></div>
        </div>
      </div>
    `;
    mountFilters(panel, state, filters);
    if (cachedRuns) paintList(panel, state, filters);
    else panel.querySelector("#history_list")!.innerHTML = '<p class="msg" role="status">正在读取运行记录…</p>';
    const runs = await loadHistory();
    if (!scope.current() || epoch !== historyEpoch) return;
    // Keep expanded cards when an unchanged background response arrives.
    if (runs !== state.allRuns) {
      state.allRuns = runs;
      paintList(panel, state, filters);
    }
  } catch (e) {
    if (!scope.current() || epoch !== historyEpoch) return;
    if (cachedRuns) toastError("历史更新失败", String(e));
    else panel.querySelector("#history_list")!.innerHTML = `<p class="msg err">${escText(String(e))}</p>`;
  }
}
