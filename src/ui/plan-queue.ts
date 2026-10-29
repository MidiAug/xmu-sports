import { invoke } from "@tauri-apps/api/core";
import { sports } from "../lib/catalog";
import { localDateISO } from "../lib/time";
import type { BookingPlan } from "../pages/booking-plans";

export async function mountPlanQueue(host: HTMLElement, current: () => boolean) {
  try {
    const view = await invoke<{ plans: { rules: BookingPlan[]; paused: boolean; attempts: { status: string; task: { target_date: string; plan_id?: string; sport: string; target_time_start: string; target_time_end: string } }[] }; next: { id: string; date: string; fire_at: string }[] }>("get_booking_plans");
    if (!current() || !host.isConnected || !view.plans.rules.some(r => r.enabled)) return;
    const active = view.plans.attempts.find(a => a.status === "running");
    const date = active?.task.target_date ?? view.next.map(n => n.date).sort()[0];
    const rows = view.plans.rules.filter(r => r.id === active?.task.plan_id || view.next.some(n => n.id === r.id && n.date === date));
    host.replaceChildren();
    const heading = document.createElement("h3"); heading.textContent = view.plans.paused ? "长期计划已暂停" : `${date} 运动的预约队列`;
    host.append(heading);
    const frozen = view.plans.attempts.filter(a => a.task.target_date === date && ["queued","running"].includes(a.status));
    const lines = frozen.length ? frozen.map(a => `${a.task.sport} ${a.task.target_time_start}–${a.task.target_time_end}${a.status === "running" ? " · 正在准备 / 执行" : " · 等待执行"}`)
      : rows.map(r => `${sports.find(s => s.id === r.sport)?.label ?? r.sport} ${r.start}–${r.end} · 等待执行`);
    lines.forEach((text,index) => {const row=document.createElement("p");row.textContent=`${index+1}. ${text}`;host.append(row);});
    const fire = view.next.find(n => n.date === date)?.fire_at;
    if (!view.plans.paused && (active || fire?.slice(0, 10) === localDateISO(new Date()))) {
      const stop = document.createElement("button"); stop.className = "btn ghost"; stop.textContent = "停止本轮计划";
      stop.onclick = async () => { stop.disabled = true; try { await invoke("stop_booking_round"); } catch (e) { const message = document.createElement("p"); message.textContent = String(e); host.append(message); stop.disabled = false; } };
      host.append(stop);
    }
  } catch { /* The regular booking view remains usable when plans cannot be read. */ }
}
