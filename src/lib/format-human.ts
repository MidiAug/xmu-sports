import type { BookingTask } from "../types";
import { localDateISO } from "./time";

/** 并列人话片段：A、B、C（跳过空段；禁止中点 ·） */
export function joinParts(...parts: Array<string | null | undefined>): string {
  return parts
    .map((p) => (p ?? "").trim())
    .filter(Boolean)
    .join("、");
}

/** @deprecated 使用 joinParts；保留别名避免漏改 */
export const joinDot = joinParts;

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** `2026-09-22` → `9月22日` */
export function formatMonthDay(isoDate: string): string {
  const m = isoDate.trim().match(/^(\d{4})-(\d{2})-(\d{2})/);
  if (!m) return isoDate;
  return `${Number(m[2])}月${Number(m[3])}日`;
}

/** Booking date options include the weekday without depending on the PC timezone. */
export function formatMonthDayWeekday(isoDate:string):string {
  const day=new Date(`${isoDate}T00:00:00Z`).getUTCDay();
  return Number.isNaN(day) ? formatMonthDay(isoDate) : `${formatMonthDay(isoDate)}(周${"日一二三四五六"[day]})`;
}

/**
 * 预约日人话：`明天 9月24日`（相对词与月日用空格，禁止中点；不展示周几）
 */
export function formatTargetDateHuman(isoDate: string, now = new Date()): string {
  const m = isoDate.trim().match(/^(\d{4})-(\d{2})-(\d{2})/);
  if (!m) return isoDate;
  const d = new Date(`${isoDate}T00:00:00+08:00`);
  const dayDiff = Math.round(
    (startOfLocalDay(d) - startOfLocalDay(now)) / 86_400_000,
  );
  const md = formatMonthDay(isoDate);
  let rel = "";
  if (dayDiff === 0) rel = "今天";
  else if (dayDiff === 1) rel = "明天";
  else if (dayDiff === 2) rel = "后天";
  else if (dayDiff === -1) rel = "昨天";
  return rel ? `${rel} ${md}` : md;
}

function startOfLocalDay(d: Date): number {
  return Date.parse(`${localDateISO(d)}T00:00:00+08:00`);
}

function parseMaybeIso(iso: string): Date | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return new Date(t);
}

/** 与 Rust arm waiter 一致：`now >= fire_at` 视为已过。 */
export function isFireAtPast(fireAt: string, now = new Date()): boolean {
  const d = parseMaybeIso(fireAt);
  if (!d) return false;
  return d.getTime() <= now.getTime();
}

/** 开抢时刻人话，禁止裸 ISO */
export function formatFireAtHuman(fireAt: string, now = new Date()): string {
  const d = parseMaybeIso(fireAt);
  if (!d) return "按任务开抢时间等待";
  const china = new Date(d.getTime() + 8 * 3600_000);
  const hm = `${pad2(china.getUTCHours())}:${pad2(china.getUTCMinutes())}${china.getUTCSeconds() ? `:${pad2(china.getUTCSeconds())}` : ""}`;
  const dayDiff = Math.round(
    (startOfLocalDay(d) - startOfLocalDay(now)) / 86_400_000,
  );
  if (dayDiff === 0) return `今天 ${hm} 开抢`;
  if (dayDiff === 1) return `明天 ${hm} 开抢`;
  if (dayDiff === -1) return `昨天 ${hm} 开抢`;
  return `${formatMonthDay(
    localDateISO(d),
  )} ${hm} 开抢`;
}

/**
 * 空闲挂机副文。
 * 开抢已过：诚实说明点挂机会立刻完整预约（禁止再写「等到…」）。
 */
export function formatFireAtWaitDesc(fireAt: string, now = new Date()): string {
  const d = parseMaybeIso(fireAt);
  if (!d) return "按任务中的开抢时间等待";
  if (isFireAtPast(fireAt, now)) {
    return "开抢时间已过，点此会马上开始预约";
  }
  const human = formatFireAtHuman(fireAt, now);
  return `等到${human.replace(/开抢$/, "")}后自动开抢`;
}

export function formatTaskSportVenue(t: BookingTask): string {
  return joinParts(t.sport, t.venue);
}

/** `9月22日 16:30–18:00`（日期与时段用空格，不用顿号） */
export function formatTargetBrief(t: BookingTask): string {
  return `${formatMonthDay(t.target_date)} ${t.target_time_start}–${t.target_time_end}`;
}

export function formatFallbackLabel(t: BookingTask | null): string {
  if (!t) return "—";
  if (!t.allow_time_fallback) return "邻近关";
  const order = t.time_fallback_order === "up_then_down" ? "先更早" : "先更晚";
  return `邻近开（${order}）`;
}

/** 事件时间：今天/昨天/月日 + 时分 */
export function formatEventTimeHuman(ts: string, now = new Date()): string {
  const d = parseMaybeIso(ts);
  if (!d) return ts;
  const china = new Date(d.getTime() + 8 * 3600_000);
  const hm = `${pad2(china.getUTCHours())}:${pad2(china.getUTCMinutes())}`;
  const dayDiff = Math.round(
    (startOfLocalDay(d) - startOfLocalDay(now)) / 86_400_000,
  );
  if (dayDiff === 0) return `今天 ${hm}`;
  if (dayDiff === -1) return `昨天 ${hm}`;
  if (dayDiff === 1) return `明天 ${hm}`;
  return `${formatMonthDay(
    localDateISO(d),
  )} ${hm}`;
}

const PAGE_ZH: Record<string, string> = {
  home: "体育首页",
  booking_notice: "公告",
  venue_sport_list: "选择运动",
  venue_list: "选择场馆",
  venue_detail: "选择日期",
  select_slot: "选择场次",
  confirm_order: "确认订单",
  success: "预约成功",
  my_reservations: "我的预约",
  xmu_login_gate: "登录页",
  xmu_wx_authorize: "微信授权",
  xmu_auth_waiting: "等待授权",
};

export function pageLabelZh(pageId: string | null | undefined): string | null {
  if (!pageId) return null;
  return PAGE_ZH[pageId] ?? null;
}

const ACTION_ZH: Record<string, string> = {
  spawn: "启动",
  spawn_failed: "启动失败",
  launch_mp: "打开小程序",
  launch_mp_failed: "打开小程序失败",
  glance: "识别页面",
  glance_failed: "识别失败",
  click: "点击",
  after_click: "点击后",
  click_failed: "点击失败",
  done: "结束",
  fail: "失败",
  cancelled: "已取消",
  started: "已开始",
};

export function actionLabelZh(action: string | null | undefined): string | null {
  if (!action) return null;
  if (ACTION_ZH[action]) return ACTION_ZH[action];
  if (action.endsWith("_failed")) return "步骤失败";
  return null;
}

/** 步骤时间线一行：动作 + 页面（皆人话） */
export function formatStepLine(ev: {
  action?: string | null;
  page_id?: string | null;
  result?: string | null;
}): string {
  if (["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "ORDER_RECHECK_UNCONFIRMED"].includes(ev.result ?? "")) return "查单待确认";
  return joinParts(actionLabelZh(ev.action), pageLabelZh(ev.page_id)) || "步骤";
}

/** 剥掉后端 `现在：` / `下一步：` 前缀，留给 UI 标签 */
export function stripGlanceLinePrefix(
  line: string | null | undefined,
  kind: "now" | "next",
): string {
  const raw = (line ?? "").trim();
  if (!raw) return "—";
  if (kind === "now") return raw.replace(/^现在\s*[：:]\s*/, "") || "—";
  return raw.replace(/^下一步\s*[：:]\s*/, "") || "—";
}

/** Shadow / 观察噪声码：核对页不应展示 */
const REFUSE_HIDE = new Set([
  "OBSERVE_ONLY_PENDING_PAGE_MAP",
  "SHADOW_SLOT_READY",
  "SHADOW_NO_SUBMIT",
]);

const REFUSE_ZH: Record<string, string> = {
  OVERLAY_BLOCKS_ACTION: "有弹层挡住",
  UNKNOWN_PAGE: "认不出页面",
  NO_AVAILABLE_SLOTS: "暂无场次",
  SLOT_GRID_NOT_LOCALIZED: "还没定位到场次格",
  CONFIRM_CTA_NOT_LOCALIZED: "底部确认预约未定位",
  NO_SAFE_CLICK_TARGET: "找不到可点目标",
  INPUT_DISABLED: "当前不可点击",
  LOW_CONFIDENCE: "把握不够",
  ANCHOR_MOVED: "目标挪了位置",
  USER_BUSY: "微信正被占用",
  LEASE_REBIND_REQUIRED: "需要重新绑窗",
  CAPTCHA_REFRESHED: "验证码已刷新，请再点唤起键盘",
};

/**
 * 核对页「下一步」附加说明。
 * 可点时不展示 refuse；噪声码隐藏；其余人话，未知码不原样露出。
 */
export function glanceRefuseHint(
  code: string | null | undefined,
  canClick: boolean,
): string | null {
  if (canClick || !code) return null;
  const c = code.trim();
  if (!c || REFUSE_HIDE.has(c)) return null;
  return REFUSE_ZH[c] ?? null;
}
