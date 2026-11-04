import { invoke } from "@tauri-apps/api/core";
import type { BookingPlansView } from "../lib/booking-plans";
import { formatMonthDayWeekday } from "../lib/format-human";
import { groupEventsIntoRuns } from "../lib/run-groups";
import type { LiveStatus, RunEvent } from "../types";
import { patchHtml } from "./patch-html";
import { sportIcon } from "./sport-icons";
import { SVG } from "./icons";

const escape = (value: string) => value.replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]!));
const labels: Record<string, string> = {
  succeeded: "已约到", existing: "已有预约", failed: "未约到", unconfirmed: "待确认",
  queued: "待执行", running: "执行中", skipped: "已跳过", missed: "已错过",
};
type RoundState = { key: string; open: Set<string>; content: Map<string, string>; outcomes: Map<string, string>; trace?: string; history?: Promise<RunEvent[]> };
const states = new WeakMap<Element, RoundState>();

/** A result belongs to its frozen round, never the next editable draft or another account. */
export function renderBookingRound(
  host: HTMLElement, status: LiveStatus | null, view: BookingPlansView | undefined,
  renderExecution: (status: LiveStatus | null) => string,
): boolean {
  if (status?.running || !view || ["auth", "prepare", "recheck"].includes(status?.operation ?? "")
    || ["http_auth_ready", "http_prepared"].includes(status?.last_event?.result ?? "")) return false;
  const anchor = view.plans.attempts.find(a => a.task.task_id === status?.last_event?.task_id);
  if (!anchor) return false;
  const items = view.plans.attempts.filter(a => a.account === anchor.account
    && a.task.target_date === anchor.task.target_date
    && Date.parse(a.task.fire_at) === Date.parse(anchor.task.fire_at));
  if (items.length < 2) return false;
  const key = JSON.stringify([anchor.account, anchor.task.target_date, anchor.task.fire_at]);
  let state = states.get(host);
  if (state?.key !== key) {
    state = {key, open:new Set(), content:new Map(), outcomes:new Map()};
    states.set(host, state);
  }
  const round = state;
  const last = status!.last_event!;
  const trace = JSON.stringify([last.task_id, last.run_id, last.ts, last.action, last.result]);
  if (round.trace !== trace) {
    round.content.delete(last.task_id);
    round.trace = trace;
  }
  for (const item of items) {
    const id = item.task.task_id;
    if (round.outcomes.get(id) !== item.status) {
      round.content.delete(id);
      round.history = undefined;
      round.outcomes.set(id, item.status);
    }
  }
  host.querySelectorAll<HTMLDetailsElement>(".round-item").forEach(details => {
    const id = details.dataset.roundTask!;
    if (details.open) round.open.add(id); else round.open.delete(id);
    const body = details.querySelector(".round-item-detail");
    if (body?.childElementCount && round.content.has(id)) round.content.set(id, body.innerHTML);
  });
  const success = items.filter(a => ["succeeded", "existing"].includes(a.status)).length;
  const failed = items.filter(a => a.status === "failed").length;
  const uncertain = items.filter(a => a.status === "unconfirmed").length;
  const pending = items.filter(a => ["queued", "running"].includes(a.status)).length;
  const title = uncertain ? "有预约待确认" : pending ? "本轮尚未结束"
    : success === items.length ? "全部约到" : success && failed ? "部分约到" : failed === items.length ? "本轮未约到" : "本轮预约已结束";
  const tone = uncertain || pending ? "pending" : success === items.length ? "success" : success ? "partial" : "finished";
  patchHtml(host, `<section class="booking-round ${tone}" aria-label="本轮预约结果">
    <header class="round-heading"><div><span class="booking-eyebrow">本轮预约结果</span><h3>${title}</h3><p>${escape(formatMonthDayWeekday(anchor.task.target_date))} · ${items.length} 项运动</p></div><span class="round-count"><strong>${success}</strong><span>/ ${items.length} 项已约到</span></span></header>
    <div class="round-totals">${success ? `<span class="round-ok">已约到 ${success} 项</span>` : ""}${failed ? `<span class="round-failed">未约到 ${failed} 项</span>` : ""}${uncertain ? `<span>待确认 ${uncertain} 项</span>` : ""}${pending ? `<span>待执行 ${pending} 项</span>` : ""}</div>
    <div class="round-items">${items.map(a => `<details class="round-item" name="booking-round-items" data-round-task="${escape(a.task.task_id)}" ${round.open.has(a.task.task_id) ? "open" : ""}>
      <summary aria-label="${escape(a.task.sport)}，${labels[a.status] ?? "已结束"}，执行详情"><span class="sport-icon round-sport" data-sport="${escape(a.task.sport)}" aria-hidden="true">${sportIcon(a.task.sport)}</span><span class="round-item-copy"><strong>${escape(a.task.sport)}</strong><span>${escape(a.task.target_time_start)}–${escape(a.task.target_time_end)}</span><small>${escape(a.task.venue)}</small></span><span class="round-item-result" data-result="${escape(a.status)}">${labels[a.status] ?? "已结束"}</span><span class="round-chevron" aria-hidden="true">${SVG.chevron}</span></summary>
      <div class="round-item-detail">${round.open.has(a.task.task_id) ? round.content.get(a.task.task_id) ?? "" : ""}</div></details>`).join("")}</div>
    <footer>展开运动查看执行详情<button type="button" class="btn text" data-tab="history">全部记录 ${SVG.arrowUpRight}</button></footer>
  </section>`);
  host.querySelectorAll<HTMLDetailsElement>(".round-item").forEach(details => {
    const updateDetails = async () => {
      const taskId = details.dataset.roundTask!;
      const body = details.querySelector<HTMLElement>(".round-item-detail")!;
      if (!details.open) {
        round.open.delete(taskId);
        if (body.childElementCount && round.content.has(taskId)) round.content.set(taskId, body.innerHTML);
        body.replaceChildren();
        return;
      }
      round.open.add(taskId);
      if (round.content.has(taskId)) { body.innerHTML = round.content.get(taskId)!; return; }
      body.textContent = "正在读取执行记录…";
      try {
        let execution = "";
        if (status?.last_event?.task_id === taskId) execution = renderExecution({...status, task:null, operation:"booking"});
        if (!execution) {
          round.history ??= invoke<RunEvent[]>("list_recent_events", {limit:0}).catch(error => { round.history = undefined; throw error; });
          const runs = groupEventsIntoRuns((await round.history).filter(e => e.task_id === taskId));
          const latest = runs.sort((a,b) => Date.parse(b.endedAt) - Date.parse(a.endedAt))[0];
          if (latest) execution = renderExecution({task:null, last_event:latest.events.at(-1)!, recent_events:latest.events, running:false, armed:false, fire_at:null, operation:"booking"});
        }
        const html = execution || '<p class="round-detail-note">未找到本项过程记录，预约结果仍按已保存记录展示。</p>';
        round.content.set(taskId, html);
        if (body.isConnected && details.open) body.innerHTML = html;
      } catch {
        if (body.isConnected && details.open) body.innerHTML = '<p class="round-detail-note" role="status">执行记录读取失败，收起后展开可重试。</p>';
      }
    };
    details.ontoggle = updateDetails;
    if (details.open && !round.content.has(details.dataset.roundTask!)) void updateDetails();
  });
  return true;
}
