import { invoke } from "@tauri-apps/api/core";
import { sports } from "../lib/catalog";
import { localDateISO } from "../lib/time";
import { formatEventTimeHuman, formatMonthDayWeekday } from "../lib/format-human";
import { hasPlanActivity, nextPlanRound, type BookingPlansView } from "../lib/booking-plans";

export async function mountPlanQueue(host: HTMLElement, current: () => boolean): Promise<boolean> {
  try {
    const view = await invoke<BookingPlansView>("get_booking_plans");
    if (!current() || !host.isConnected) return false;
    return renderPlanQueue(host, view);
  } catch (error) {
    if (current() && host.isConnected) {
      host.replaceChildren();
      const message = document.createElement("p");
      message.textContent = `暂未读取到长期计划：${String(error)}`;
      host.append(message);
    }
    return true;
  }
}

export function renderPlanQueue(host: HTMLElement, view: BookingPlansView): boolean {
  host.replaceChildren();
  if (!hasPlanActivity(view)) return false;
  const round = nextPlanRound(view);
  const pending = view.plans.attempts.filter(a => a.status === "unconfirmed");
  host.innerHTML = `<div class="plan-queue-heading"><div><span class="section-eyebrow">长期计划</span><h3></h3></div><button type="button" class="btn text" data-tab="plans">管理计划</button></div><p class="plan-queue-schedule"></p><ol class="plan-queue-items"></ol><div class="plan-queue-actions"></div>`;
  host.querySelector("h3")!.textContent = pending.length ? "有预约结果需要核对" : view.plans.paused ? "自动计划已暂停" : round?.items.some(i => i.status === "running") ? "本轮预约队列" : "下一轮自动预约";
  host.querySelector(".plan-queue-schedule")!.textContent = pending.length ? `${pending.length} 项结果待确认，请先处理后再恢复计划` : round ? `${view.plans.paused ? "恢复后按计划 " : ""}${formatEventTimeHuman(round.fireAt)} 开抢，预约 ${formatMonthDayWeekday(round.date)} 的运动` : "暂无待执行项目";
  const list = host.querySelector("ol")!;
  round?.items.forEach((item, index) => {
    const sport = sports.find(s => s.id === item.sport);
    const row = document.createElement("li");
    row.className = "plan-queue-row";
    row.innerHTML = `<span class="plan-priority">${index + 1}</span><div class="plan-queue-target"><strong></strong><span></span></div><span class="plan-queue-time"></span><span class="plan-queue-status"></span>`;
    row.querySelector("strong")!.textContent = sport?.label ?? item.sport;
    row.querySelector(".plan-queue-target > span")!.textContent = sport?.venues.find(v => v.id === item.venue)?.label ?? item.venue;
    row.querySelector(".plan-queue-time")!.textContent = `${item.start}–${item.end}`;
    row.querySelector(".plan-queue-status")!.textContent = item.status === "running" ? "准备 / 执行中" : view.plans.paused ? "恢复后执行" : item.status === "queued" ? "等待执行" : "已安排";
    list.append(row);
  });
  const actions = host.querySelector(".plan-queue-actions")!;
  if (pending.length) {
    const open = document.createElement("button");
    open.className = "btn"; open.dataset.tab = "plans"; open.textContent = "处理待确认预约";
    actions.append(open);
  } else if (!view.plans.paused && round && (round.items.some(i => i.status === "running" || i.status === "queued") || round.fireAt.slice(0, 10) === localDateISO(new Date()))) {
    const stop = document.createElement("button"); stop.className = "btn ghost"; stop.textContent = "停止本轮计划";
    stop.onclick = async () => {
      stop.disabled = true;
      try { await invoke("stop_booking_round"); }
      catch (error) { const message = document.createElement("p"); message.textContent = String(error); actions.append(message); stop.disabled = false; }
    };
    actions.append(stop);
  }
  return true;
}
