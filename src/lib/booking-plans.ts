import type { BookingTask } from "../types";

export type BookingPlan = {
  id: string; sport: string; venue: string; start: string; end: string;
  weekdays: number[]; enabled: boolean; account: string; first_date: string;
};
export type BookingPlanAttempt = {
  task: BookingTask; sport: string; account: string; status: string; message: string;
};
export type BookingPlansView = {
  plans: { rules: BookingPlan[]; paused: boolean; attempts: BookingPlanAttempt[] };
  next: { id: string; date: string; fire_at: string }[];
  wechat_running: boolean;
};
export const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"];
export function weekdaysLabel(values: number[]) {
  if (values.length === 7) return "每天";
  if (values.join() === "1,2,3,4,5") return "工作日";
  if (values.join() === "6,7") return "周末";
  return "每周" + values.map(day => WEEKDAYS[day - 1]).join("、");
}

/** Frozen tasks retain their original targets and order when rules are edited. */
export function nextPlanRound(view: BookingPlansView) {
  const frozen = view.plans.attempts.filter(a => a.status === "queued" || a.status === "running");
  const active = frozen.find(a => a.status === "running") ?? frozen[0];
  const next = [...view.next].sort((a, b) => a.fire_at.localeCompare(b.fire_at))[0];
  const date = active?.task.target_date ?? next?.date;
  if (!date) return null;
  const items = active ? frozen.filter(a => a.task.target_date === date).map(a => ({
    id: a.task.plan_id, sport: a.task.sport, venue: a.task.venue,
    start: a.task.target_time_start, end: a.task.target_time_end, status: a.status,
  })) : view.plans.rules.filter(rule => rule.enabled && view.next.some(n => n.id === rule.id && n.date === date)).map(rule => ({
    id: rule.id, sport: rule.sport, venue: rule.venue, start: rule.start, end: rule.end, status: "upcoming",
  }));
  return { date, fireAt: active?.task.fire_at ?? next!.fire_at, items };
}

export function hasPlanActivity(view: BookingPlansView | undefined) {
  return Boolean(view && (view.plans.rules.some(r => r.enabled)
    || view.plans.attempts.some(a => ["queued", "running", "unconfirmed"].includes(a.status))));
}

/** Future openings in the visible planning horizon; a paused plan is not queued. */
export function hasUpcomingPlan(view: BookingPlansView | undefined, today: string, tomorrow: string, now: number): boolean {
  if (!view || view.plans.paused || view.plans.attempts.some(a => a.status === "unconfirmed")) return false;
  if (view.plans.attempts.some(a => a.status === "queued" || a.status === "running")) return true;
  return view.next.some(next => [today,tomorrow].includes(next.fire_at.slice(0,10)) && Date.parse(next.fire_at) >= now
    && view.plans.rules.some(rule => rule.id === next.id && rule.enabled));
}
