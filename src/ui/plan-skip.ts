import { invoke } from "@tauri-apps/api/core";
import { bookingPlans } from "../lib/booking-plans-store";
import { formatMonthDayWeekday } from "../lib/format-human";
import { sports } from "../lib/catalog";
import { toast, toastError, toastSuccess } from "./toast";

function occurrenceLabel(id: string, date: string) {
  const rule = bookingPlans.snapshot().view?.plans.rules.find(rule => rule.id === id);
  const sport = sports.find(sport => sport.id === rule?.sport)?.label;
  return `${sport ? `${sport}，` : ""}${formatMonthDayWeekday(date)} 的运动预约`;
}

export async function restorePlanSkip(id: string, date: string) {
  await invoke("restore_booking_plan", { id, date });
  toastSuccess("已恢复本次", occurrenceLabel(id, date));
  await bookingPlans.read(true).catch(error => toastError("计划暂未更新", String(error)));
}

export async function skipPlanOnce(id: string) {
  const date = await invoke<string>("skip_booking_plan", { id });
  toast({ kind: "success", title: "已跳过本次", body: occurrenceLabel(id, date), durationMs: 8000,
    action: { label: "撤销", run: () => restorePlanSkip(id, date) } });
  await bookingPlans.read(true).catch(error => toastError("计划暂未更新", String(error)));
}
