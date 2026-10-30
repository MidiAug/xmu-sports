import { toastError } from "./toast";
import { invoke } from "@tauri-apps/api/core";
import type { LiveStatus } from "../types";
import { sports } from "../lib/catalog";
import { localDateISO } from "../lib/time";
import { formatEventTimeHuman, formatMonthDayWeekday } from "../lib/format-human";
import { nextPlanRound, type BookingPlansView } from "../lib/booking-plans";
import { patchHtml } from "./patch-html";
import { sportIcon } from "./sport-icons";
import { SVG } from "./icons";

export function renderPlanQueue(host: HTMLElement, view: BookingPlansView | undefined, status?: LiveStatus | null): boolean {
  const content = document.createElement("div");
  const round = view && !view.plans.paused ? nextPlanRound(view) : null;
  const pending = view?.plans.attempts.filter(a => a.status === "unconfirmed") ?? [];
  const single = status?.armed && !status.running && !status.task?.plan_id ? status.task : null;
  const items = [
    ...(round?.items ?? []).filter(item => !(status?.running && status.task?.plan_id === item.id)).map(item => ({...item,date:round!.date,fireAt:round!.fireAt,source:"计划"})),
    ...(single ? [{id:undefined,sport:single.sport,venue:single.venue,start:single.target_time_start,end:single.target_time_end,status:"upcoming",date:single.target_date,fireAt:status?.fire_at ?? single.fire_at,source:"单次"}] : []),
  ].sort((a,b)=>Date.parse(a.fireAt)-Date.parse(b.fireAt));
  const activeRound = round?.items.some(item => item.status === "running" || item.status === "queued");
  if (!items.length && !pending.length && !view?.plans.paused && !activeRound) { patchHtml(host, ""); return false; }
  content.innerHTML = `<div class="plan-queue-overview"><div class="plan-queue-heading"><div><span class="section-eyebrow">长期计划</span><h3></h3></div><button type="button" class="btn ghost live-context-action" data-tab="plans">管理计划${SVG.arrowUpRight}</button></div><p class="plan-queue-schedule"></p></div><ol class="plan-queue-items"></ol><div class="plan-queue-actions"></div>`;
  content.querySelector("h3")!.textContent = pending.length ? "预约结果待确认" : items.length ? `${formatEventTimeHuman(items[0].fireAt)} 开抢` : activeRound ? "本轮进行中" : "计划已暂停";
  content.querySelector(".section-eyebrow")!.textContent = items.length ? "下一次开抢" : pending.length ? "需要处理" : activeRound ? "正在预约" : "预约安排";
  content.querySelector(".plan-queue-schedule")!.textContent = pending.length ? "请先核对结果，确认前不会自动重复下单" : items.length ? `${items.length} 项待执行 · 按开抢时间排列` : activeRound ? "当前预约结束后完成本轮" : "恢复计划后继续自动预约";
  const list = content.querySelector("ol")!;
  if (!items.length) { list.remove(); content.querySelector(".plan-queue-overview")!.classList.add("queue-overview-only"); }
  items.forEach((item, index) => {
    const sport = sports.find(s => s.id === item.sport || s.ocr_label === item.sport);
    const row = document.createElement("li");
    row.className = "plan-queue-row";
    row.innerHTML = `<span class="plan-queue-icon sport-icon">${sportIcon(sport?.label ?? item.sport)}<small>${index + 1}</small></span><div class="plan-queue-target"><strong></strong><span></span></div><span class="plan-queue-time"></span><span class="plan-queue-status"></span>`;
    row.querySelector<HTMLElement>(".plan-queue-icon")!.dataset.sport = sport?.label ?? item.sport;
    row.querySelector("strong")!.textContent = sport?.label ?? item.sport;
    row.querySelector(".plan-queue-target > span")!.textContent = `${sport?.venues.find(v => v.id === item.venue || v.ocr_label === item.venue)?.label ?? item.venue} · ${formatMonthDayWeekday(item.date)}`;
    row.querySelector(".plan-queue-time")!.textContent = `${item.start}–${item.end}`;
    const source = document.createElement("button"); source.className="queue-source"; source.textContent=item.source;
    source.dataset.tab=item.source === "计划" ? "plans" : "setup";
    if(item.id) source.dataset.planId=item.id;
    row.querySelector(".plan-queue-status")!.replaceChildren(source,document.createTextNode(` · ${`${formatEventTimeHuman(item.fireAt).replace(/ 开抢$/, "")} 开抢`}`));
    list.append(row);
  });
  const actions = content.querySelector(".plan-queue-actions")!;
  if (single) {
    const cancel = document.createElement("button"); cancel.className="btn ghost"; cancel.id="disarm_hang"; cancel.textContent="取消单次定时"; actions.append(cancel);
  }
  if (pending.length) {
    const open = document.createElement("button");
    open.className = "btn"; open.dataset.tab = "plans"; open.textContent = "处理待确认预约";
    actions.append(open);
  } else if (!view?.plans.paused && round && (round.items.some(i => i.status === "running" || i.status === "queued") || round.fireAt.slice(0, 10) === localDateISO(new Date()))) {
    const stop = document.createElement("button"); stop.className = "btn ghost"; stop.textContent = "停止本轮计划"; stop.dataset.stopRound = "";
    actions.append(stop);
  }
  patchHtml(host, content.innerHTML);
  const stop = host.querySelector<HTMLButtonElement>("[data-stop-round]");
  if (stop) {
    stop.onclick = async () => {
      stop.disabled = true;
      try { await invoke("stop_booking_round"); }
      catch (error) { const message = document.createElement("p"); message.textContent = String(error); host.querySelector(".plan-queue-actions")!.append(message); stop.disabled = false; }
    };
  }
  const cancel = host.querySelector<HTMLButtonElement>("#disarm_hang");
  if (cancel) cancel.onclick = async () => {
    cancel.disabled = true;
    try { await invoke("disarm_hang"); }
    catch (error) { toastError("取消失败",String(error)); cancel.disabled = false; }
  };
  return items.length > 0 || pending.length > 0 || !!view?.plans.paused;
}
