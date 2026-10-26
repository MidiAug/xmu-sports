export function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

export function localDateISO(d: Date): string {
  return new Date(d.getTime() + 8 * 3600_000).toISOString().slice(0, 10);
}

export function parseLocalDateISO(iso: string): Date {
  const [y, m, day] = iso.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, day) - 8 * 3600_000);
}

export function addDaysISO(iso: string, days: number): string {
  const d = parseLocalDateISO(iso);
  d.setUTCDate(d.getUTCDate() + days);
  return localDateISO(d);
}

export function todayISO(): string {
  return localDateISO(new Date());
}

/** Next 07:00 in China, independent of the computer's timezone. */
export function nextSevenAm(now = new Date()): Date {
  const next = new Date(`${localDateISO(now)}T07:00:00+08:00`);
  if (now.getTime() >= next.getTime()) {
    next.setUTCDate(next.getUTCDate() + 1);
  }
  return next;
}

/** Default booking day = calendar day after the next 7:00 grab window. */
export function defaultTargetDateISO(now = new Date()): string {
  const d = nextSevenAm(now);
  d.setUTCDate(d.getUTCDate() + 1);
  return localDateISO(d);
}

/** The booking form only offers today and the following two days in Beijing. */
export function bookingDateOptions(now = new Date()): { value: string; label: string }[] {
  const today = localDateISO(now);
  return ["今天", "明天", "后天"].map((label, offset) => ({
    value: addDaysISO(today, offset),
    label,
  }));
}

export function isSelectableTargetDate(iso: string, now = new Date()): boolean {
  return bookingDateOptions(now).some(option => option.value === iso);
}

export function waitsExtraDay(targetDate: string, now = new Date()): boolean {
  return new Date(now.getTime() + 8 * 3600_000).getUTCHours() < 7
    && targetDate === addDaysISO(localDateISO(now), 2);
}

export function targetDateHint(now = new Date(), targetDate?: string): string {
  if (targetDate && waitsExtraDay(targetDate, now))
    return "后天的名额需等到明天 07:00；若要参加今天 07:00 的这一轮，请选择明天，避免多等一天。";
  const hour = new Date(now.getTime() + 8 * 3600_000).getUTCHours();
  if (hour < 7) {
    return "默认预约明天，今天 07:00 开抢";
  }
  return "默认预约后天，明天 07:00 开抢";
}

export const FIRE_TIME_MIN = "07:00";

export function defaultFireTimeHm(): string {
  return FIRE_TIME_MIN;
}

export function hmToMinutes(hm: string): number {
  const [h, m] = hm.slice(0, 5).split(":").map(Number);
  return h * 60 + (m || 0);
}

export function minutesToHm(total: number): string {
  const t = Math.max(0, Math.min(23 * 60 + 59, total));
  return `${pad2(Math.floor(t / 60))}:${pad2(t % 60)}`;
}

export function addMinutesHm(hm: string, delta: number, minHm = FIRE_TIME_MIN): string {
  const next = hmToMinutes(hm) + delta;
  const min = hmToMinutes(minHm);
  const max = 23 * 60 + 59;
  return minutesToHm(Math.min(max, Math.max(min, next)));
}

export function fireHmFromIso(fireAt: string, delaySeconds = 0): string {
  const timestamp = Date.parse(fireAt);
  const base = Number.isFinite(timestamp)
    ? new Date(timestamp - delaySeconds * 1000 + 8 * 3600_000).toISOString()
    : fireAt;
  const m = base.match(/T(\d{2}:\d{2})/);
  if (!m) return defaultFireTimeHm();
  return hmToMinutes(m[1]) < hmToMinutes(FIRE_TIME_MIN) ? FIRE_TIME_MIN : m[1];
}

export function tzOffsetSuffix(d: Date): string {
  const off = -d.getTimezoneOffset();
  const sign = off >= 0 ? "+" : "-";
  return `${sign}${pad2(Math.floor(Math.abs(off) / 60))}:${pad2(Math.abs(off) % 60)}`;
}

/** School scheduling always uses Beijing time. */
export function composeFireAt(targetDate: string, timeHm: string, delaySeconds = 0): string {
  if (!Number.isInteger(delaySeconds) || delaySeconds < 0 || delaySeconds > 300)
    throw new Error("执行延迟须为 0–300 的整数秒");
  const hm = timeHm.slice(0, 5);
  const [hh, mm] = hm.split(":").map(Number);
  const fireDate = addDaysISO(targetDate, -1);
  const base = Date.parse(`${fireDate}T${pad2(hh)}:${pad2(mm || 0)}:00+08:00`);
  return new Date(base + delaySeconds * 1000 + 8 * 3600_000).toISOString().slice(0, 19) + "+08:00";
}

/** Before 07:00, today's target waits for today's window rather than yesterday's. */
export function plannedFireAt(targetDate: string, timeHm: string, delaySeconds = 0, now = new Date()): string {
  const today = localDateISO(now);
  const beforeSeven = new Date(now.getTime() + 8 * 3600_000).getUTCHours() < 7;
  const scheduleTarget = beforeSeven && addDaysISO(targetDate, -1) < today
    ? addDaysISO(today, 1) : targetDate;
  return composeFireAt(scheduleTarget, timeHm, delaySeconds);
}

export function formatRemaining(ms: number): string {
  if (ms <= 0) return "即将开抢";
  const totalSec = Math.floor(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) return `${h} 小时 ${pad2(m)} 分 ${pad2(s)} 秒`;
  if (m > 0) return `${m} 分 ${pad2(s)} 秒`;
  return `${s} 秒`;
}
