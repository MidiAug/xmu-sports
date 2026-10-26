import type { BookingTask } from "../types";

/** School booking uses China time, independently of the computer's timezone. */
export function bookingBlockedReason(task: BookingTask, now = new Date()): string | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(task.target_date) ||
      !/^([01]\d|2[0-3]):[0-5]\d$/.test(task.target_time_start) ||
      !/^([01]\d|2[0-3]):[0-5]\d$/.test(task.target_time_end))
    return "预约日期或时段无效，请重新选择";
  const china = new Date(now.getTime() + 8 * 3600_000);
  const today = china.toISOString().slice(0, 10);
  const tomorrow = new Date(china.getTime() + 86400_000).toISOString().slice(0, 10);
  if (task.target_date !== today && task.target_date !== tomorrow)
    return "立即预约只支持今天或明天；其他日期请等待对应开抢时间";
  const start = Date.parse(`${task.target_date}T${task.target_time_start}:00+08:00`);
  const end = Date.parse(`${task.target_date}T${task.target_time_end}:00+08:00`);
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start)
    return "预约时段无效，请重新选择";
  if (start <= now.getTime()) return "目标时段已经开始或已结束，请选择尚未开始的时段";
  return null;
}

export function hangBlockedReason(task: BookingTask, now = new Date()): string | null {
  const fire = new Date(task.fire_at);
  if (!Number.isFinite(fire.getTime())) return "开抢时间无效，请重新设置";
  return bookingBlockedReason(task, fire > now ? fire : now);
}
