import { invoke } from "@tauri-apps/api/core";
import { sports } from "../lib/catalog";
import { localDateISO } from "../lib/time";
import { formatEventTimeHuman, formatMonthDayWeekday } from "../lib/format-human";
import { hasPlanActivity, nextPlanRound, type BookingPlansView } from "../lib/booking-plans";
import { patchHtml } from "./patch-html";
import { sportIcon } from "./sport-icons";

export function renderPlanQueue(host: HTMLElement, view: BookingPlansView): boolean {
  if (!hasPlanActivity(view)) { patchHtml(host, ""); return false; }
  const content = document.createElement("div");
  const round = nextPlanRound(view);
  const roundSports = round ? [...new Set(round.items.map(item => sports.find(s => s.id === item.sport || s.ocr_label === item.sport)?.label ?? item.sport))].join("、") : "";
  const pending = view.plans.attempts.filter(a => a.status === "unconfirmed");
  content.innerHTML = `<div class="plan-queue-overview"><div class="plan-queue-heading"><div><span class="section-eyebrow">长期计划</span><h3></h3></div><button type="button" class="btn ghost" data-tab="plans">管理计划<span aria-hidden="true">↗</span></button></div><p class="plan-queue-schedule"></p></div><ol class="plan-queue-items"></ol><div class="plan-queue-actions"></div>`;
  content.querySelector("h3")!.textContent = pending.length ? "有预约结果需要核对" : view.plans.paused ? "自动计划已暂停" : round?.items.some(i => i.status === "running") ? "本轮预约队列" : "下一轮自动预约";
  content.querySelector(".plan-queue-schedule")!.textContent = pending.length ? `${pending.length} 项结果待确认，请先处理后再恢复计划` : round ? `${view.plans.paused ? "恢复后按计划 " : ""}${formatEventTimeHuman(round.fireAt)} 开抢，预约 ${formatMonthDayWeekday(round.date)} 的${roundSports}` : "暂无待执行项目";
  const list = content.querySelector("ol")!;
  round?.items.forEach((item, index) => {
    const sport = sports.find(s => s.id === item.sport);
    const row = document.createElement("li");
    row.className = "plan-queue-row";
    row.innerHTML = `<span class="plan-queue-icon sport-icon">${sportIcon(sport?.label ?? item.sport)}<small>${index + 1}</small></span><div class="plan-queue-target"><strong></strong><span></span></div><span class="plan-queue-time"></span><span class="plan-queue-status"></span>`;
    row.querySelector<HTMLElement>(".plan-queue-icon")!.dataset.sport = sport?.label ?? item.sport;
    row.querySelector("strong")!.textContent = sport?.label ?? item.sport;
    row.querySelector(".plan-queue-target > span")!.textContent = sport?.venues.find(v => v.id === item.venue)?.label ?? item.venue;
    row.querySelector(".plan-queue-time")!.textContent = `${item.start}–${item.end}`;
    row.querySelector(".plan-queue-status")!.textContent = item.status === "running" ? "准备 / 执行中" : view.plans.paused ? "恢复后执行" : item.status === "queued" ? "等待执行" : "已安排";
    list.append(row);
  });
  const actions = content.querySelector(".plan-queue-actions")!;
  if (pending.length) {
    const open = document.createElement("button");
    open.className = "btn"; open.dataset.tab = "plans"; open.textContent = "处理待确认预约";
    actions.append(open);
  } else if (!view.plans.paused && round && (round.items.some(i => i.status === "running" || i.status === "queued") || round.fireAt.slice(0, 10) === localDateISO(new Date()))) {
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
  return true;
}
