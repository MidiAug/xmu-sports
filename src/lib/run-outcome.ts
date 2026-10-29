import type { RunEvent } from "../types";

/** 跑态结局（持久条 + toast 共用）。见 2026-09-22-app-toast-outcome-design.md */

export type OutcomeKind = "success" | "error" | "info" | "warn";

export type RunOutcome = {
  kind: OutcomeKind;
  /** 机器码，如 DATE_UNAVAILABLE / slot_all_grey */
  code: string;
  title: string;
  detail: string;
  pageId: string | null;
  ts: string | null;
};

const CODE_ZH: Record<string, { kind: OutcomeKind; title: string; detail: string }> = {
  API_REJECTED: {kind:"error",title:"学校接口拒绝请求",detail:"查看运行过程中的失败阶段和学校返回的原因"},
  http_order_rechecked: {kind:"success",title:"查单已确认原预约",detail:"原账号与实际提交场次匹配，未重复下单"},
  ORDER_RECHECK_UNCONFIRMED: {kind:"info",title:"原预约仍待确认",detail:"只读查单未完成或未找到匹配记录，未重复下单"},
  http_auth_ready: {kind: "success", title: "体育登录已就绪", detail: "登录检查通过，未提交预约"},
  LOGIN_CONFIRM_REQUIRED: {kind: "error", title: "小程序导航或登录核验未完成", detail: "请查看小程序当前页面，手动进入场馆预约后重试；尚未提交预约"},
  NO_ENTRY: {kind: "error", title: "请先确认体育登录", detail: "打开电脑微信中的厦大体育，完成授权并进入场馆预约"},
  ACCOUNT_CHANGED: {kind: "error", title: "登录账号发生变化", detail: "请确认电脑微信账号，再重新执行任务"},
  HTTP_AUTH_UNVERIFIED: {kind: "error", title: "无法确认体育登录账号", detail: "已停止本次预约，请重新检查体育登录并导出诊断日志"},
  AMBIGUOUS_PROFILE: {kind: "error", title: "无法确定当前体育账号", detail: "发现多个体育缓存账号，请确认当前电脑微信账号并导出诊断日志"},
  http_verified: {kind: "success", title: "预约成功，已查单确认", detail: "学校已接受预约，并查到匹配订单"},
  http_already_reserved: {kind: "success", title: "已有该时段预约", detail: "查到了原有订单，本次没有再次下单"},
  http_submitted: {kind: "warn", title: "已提交，尚未查单确认", detail: "学校已接受，请在我的预约确认；不要重复下单"},
  http_prepared: {kind: "success", title: "预约条件检查通过", detail: "未提交预约"},
  SLOT_OCCUPIED: {kind: "error", title: "场次名额已满", detail: "目标位置已被预约"},
  SLOT_LOCKED: {kind: "error", title: "目标场次锁定", detail: "学校不提供该场次预约"},
  SLOT_UNAVAILABLE: {kind: "error", title: "目标场次不可约", detail: "学校没有提供明确原因"},
  BOOKING_CLOSED: {kind: "error", title: "预约已截止", detail: "该场次已超过预约截止时间"},
  DATE_NOT_OPEN: {kind: "error", title: "目标日期暂未发布", detail: "目标日期不在实时预约范围"},
  NOT_OPEN: {kind: "error", title: "当前尚未开放预约", detail: "学校当前未开放该日期或场次，请刷新实时状态后重试"},
  INVALID_SLOT: {kind: "error", title: "预约时段无效", detail: "请重新选择预约日期和时段"},
  CAPTCHA_FETCH_FAILED: {kind: "error", title: "验证码获取失败", detail: "学校未返回有效验证码，请查看执行记录"},
  CAPTCHA_UNREADABLE: {kind: "error", title: "验证码识别未完成", detail: "已达到识别次数上限，请查看执行记录"},
  CAPTCHA_REJECTED: {kind: "error", title: "验证码未通过", detail: "学校未接受验证码；每个候选最多尝试 3 次（含首次），请查看执行记录"},
  FALLBACKS_EXHAUSTED: {kind: "error", title: "备选场次均不可约", detail: "已按设置检查候选时段"},
  SPORT_DAY_ALREADY_BOOKED: {kind: "info", title: "当天这项运动已有预约", detail: "已跳过本次计划，没有重复下单"},
  ACCOUNT_LIMIT: {kind: "error", title: "账号预约次数受限", detail: "学校不允许本次预约"},
  SESSION_EXPIRED: {kind: "error", title: "体育登录已失效", detail: "请在电脑微信重新授权，再执行原任务"},
  NATIVE_TOKEN_REQUIRED: {kind: "error", title: "当前登录模式尚不兼容", detail: "当前会话使用新版登录接口，请导出诊断日志"},
  RUNTIME_UNSUPPORTED: {kind: "error", title: "当前微信内核版本尚未适配自动入口", detail: "请在设置中导出诊断日志"},
  RUNTIME_PATH_UNAVAILABLE: {kind: "error", title: "无法确定微信内核位置", detail: "请重启电脑微信后重试；仍失败请在设置中导出诊断日志"},
  RUNTIME_FILE_MISSING: {kind: "error", title: "未找到微信内核文件", detail: "已找到小程序进程，请重启电脑微信后重试；仍失败请导出诊断日志"},
  RUNTIME_FILE_UNREADABLE: {kind: "error", title: "无法读取微信内核文件", detail: "请确认微信和本软件以同一用户运行，并在设置中导出诊断日志"},
  RUNTIME_AMBIGUOUS: {kind: "error", title: "无法确定正在使用的微信内核", detail: "请重启电脑微信后重试；仍失败请导出诊断日志"},
  RUNTIME_PROBE_FAILED: {kind: "error", title: "微信小程序进程检查失败", detail: "请重启电脑微信后重试，并在设置中导出诊断日志"},
  RUNTIME_ATTACH_FAILED: {kind: "error", title: "无法连接微信小程序内核", detail: "请确认微信和本软件以同一用户运行后重试；仍失败请导出诊断日志"},
  RUNTIME_UNAVAILABLE: {kind: "error", title: "未找到小程序内核", detail: "请在电脑微信打开一次厦大体育后重试"},
  RUNTIME_BUSY: {kind: "error", title: "自动入口通道被占用", detail: "请关闭小程序调试工具后重试"},
  RUNTIME_BRIDGE_FAILED: {kind: "error", title: "自动获取场馆入口未完成", detail: "请打开厦大体育确认登录状态，再进入场馆预约后重试"},
  RUNTIME_CLOSE_FAILED: {kind: "error", title: "小程序关闭未完成", detail: "新票据已验证，请关闭小程序后重试"},
  LOGIN_FOCUS_REQUIRED: {kind: "error", title: "小程序未获得操作焦点", detail: "小程序可能被微信主窗或其他窗口遮挡、抢走焦点，请切回厦大体育后重试"},
  SUBMIT_UNKNOWN: {kind: "info", title: "预约结果待确认", detail: "尚未确认是否成功，保留查单与去重记录，不会重复下单"},
  RESULT_UNCONFIRMED: {kind: "info", title: "已有预约结果待确认", detail: "请先查单，不会自动重放请求"},
  HTTP_COMPONENT_MISSING: {kind: "error", title: "预约组件缺失", detail: "请重新安装包含 HTTP 组件的新版软件"},
  HTTP_WORKER_ERROR: {kind: "error", title: "请求组件执行失败", detail: "请查看日志；若已提交请先查单"},
  HTTP_WORKER_START_FAILED: {kind: "error", title: "请求组件无法启动", detail: "请导出诊断日志"},
  HTTP_WORKER_TIMEOUT: {kind: "error", title: "请求组件执行超时", detail: "本次流程已停止，请查看诊断日志"},
  HTTP_FAILED: {kind: "error", title: "学校接口请求失败", detail: "请稍后再试；本次未继续切换候选"},
  RATE_LIMITED: {kind: "error", title: "学校接口限制请求频率", detail: "已停止本次请求，请稍后再试"},
  SERVER_BUSY: {kind: "error", title: "学校服务暂时异常", detail: "已停止本次请求，请稍后再试"},
  DATE_UNAVAILABLE: {
    kind: "error",
    title: "目标日暂未开放",
    detail: "场馆详情里还没有你选的日期，请改天再试或改任务日期",
  },
  DATE_MISMATCH: {
    kind: "error",
    title: "日期对不上",
    detail: "连续多帧确认目标日不在日期条上",
  },
  DATE_EVIDENCE_WEAK: {
    kind: "warn",
    title: "日期还看不清",
    detail: "正在多帧核对日期条，请稍候",
  },
  DATE_EVIDENCE_TIMEOUT: {
    kind: "error",
    title: "日期证据不足",
    detail: "多次刷新仍无法确认目标日，未判定为日期不匹配",
  },
  NO_AVAILABLE_SLOTS: {
    kind: "error",
    title: "没有可约场次",
    detail: "目标日场次列表为空",
  },
  SLOT_ALL_GREY: {
    kind: "success",
    title: "试跑完成：场次全暂满",
    detail: "已停在场次页（算试跑成功停点）",
  },
  slot_all_grey: {
    kind: "success",
    title: "试跑完成：场次全暂满",
    detail: "已停在场次页",
  },
  trial_reached_confirm: {
    kind: "success",
    title: "试跑完成",
    detail: "已点到可约格，停在确认前",
  },
  trial_slot_clicked: {
    kind: "success",
    title: "试跑完成",
    detail: "已点选可约场次",
  },
  trial_slot_ready: {
    kind: "success",
    title: "试跑完成",
    detail: "场次已选中，底部确认已激活（试跑不停确认）",
  },
  ANCHOR_NOT_FOUND: {
    kind: "error",
    title: "找不到要点的控件",
    detail: "页面可能还在加载，或布局有变",
  },
  UNKNOWN_PAGE: {
    kind: "error",
    title: "无法识别当前页",
    detail: "请回到小程序预约流程后再试",
  },
  READY_TIMEOUT: {
    kind: "error",
    title: "等页面就绪超时",
    detail: "小程序还像在加载中",
  },
  LAUNCH_MP_NEED_DESKTOP: {
    kind: "error",
    title: "打不开小程序窗口",
    detail:
      "请在 PC 微信中手动打开「厦大体育」，点右上角三点菜单 →「添加到电脑桌面」后再试运行",
  },
  LAUNCH_MP_PROTOCOL: {
    kind: "error",
    title: "无法唤起微信协议",
    detail: "确认已安装并登录 PC 微信",
  },
  launch_mp_failed: {
    kind: "error",
    title: "打开小程序失败",
    detail: "协议唤起后未出现小程序窗；可先「添加到电脑桌面」再试",
  },
  ALREADY_DONE: {
    kind: "success",
    title: "已经约过了",
    detail: "任务目标时段已有预约",
  },
  already_done: {
    kind: "success",
    title: "已经约过了",
    detail: "任务目标时段已有预约",
  },
  submitted: {
    kind: "success",
    title: "预约流程完成",
    detail: "已到达成功页或我的预约",
  },
  NO_CLICKABLE_STEP: {
    kind: "error",
    title: "没有可点的下一步",
    detail: "当前页无法继续安全点击",
  },
  MAX_STEPS: {
    kind: "error",
    title: "步数超限",
    detail: "视觉流程走得太久，已停止",
  },
  SLOT_GRID_NOT_LOCALIZED: {
    kind: "error",
    title: "场次格子未定位",
    detail: "无法安全点击场次",
  },
  CONFIRM_CTA_NOT_LOCALIZED: {
    kind: "error",
    title: "确认预约按钮未定位",
    detail: "场次已选中，但底部确认按钮找不到",
  },
  SLOT_TIME_MISMATCH: {
    kind: "error",
    title: "时段对不上",
    detail: "目标时段（及已开启的邻近时段）找不到可约格",
  },
  SLOT_COURT_MISMATCH: {
    kind: "error",
    title: "场地对不上",
    detail: "偏好场地不可约",
  },
  VENUE_MISMATCH: {
    kind: "error",
    title: "场馆对不上",
    detail: "列表里找不到任务场馆",
  },
  SPORT_NOT_ON_SCREEN: {
    kind: "error",
    title: "运动不在当前屏",
    detail: "运动列表里找不到目标运动",
  },
  GOAL_UNSPECIFIED: {
    kind: "error",
    title: "任务目标不完整",
    detail: "请回任务页补全运动/馆/日期/时段",
  },
  OVERLAY_BLOCKS_ACTION: {
    kind: "error",
    title: "弹层挡住了操作",
    detail: "请先关掉公告或弹窗",
  },
  cancelled: {
    kind: "info",
    title: "已取消",
    detail: "本次执行已停止",
  },
  started: {
    kind: "info",
    title: "已开始",
    detail: "执行流程启动中",
  },
  success: {
    kind: "success",
    title: "预约成功",
    detail: "已到达成功页或我的预约",
  },
  failed: {
    kind: "error",
    title: "执行失败",
    detail: "详情见运行页最近结局",
  },
};

function mapCode(code: string): { kind: OutcomeKind; title: string; detail: string } {
  const hit = CODE_ZH[code];
  if (hit) return hit;
  return {
    kind: "info",
    title: `已结束：${code}`,
    detail: code,
  };
}

/** 从 last_event 推导可展示结局；中间步骤返回 null */
export function outcomeFromEvent(ev: RunEvent | null | undefined): RunOutcome | null {
  if (!ev) return null;
  const action = ev.action ?? "";
  const result = (ev.result ?? "").trim();

  if (action === "done") {
    const code = result || "success";
    const m = mapCode(code);
    return {
      kind: m.kind === "info" ? "success" : m.kind,
      code,
      title: m.title,
      detail: ev.http?.message || (ev.http?.verified ? `${ev.http.date ?? ""} ${ev.http.time ?? ""} ${ev.http.court ? ev.http.court + " 号场地" : ""}`.trim() : m.detail),
      pageId: ev.page_id ?? null,
      ts: ev.ts ?? null,
    };
  }

  if (action === "fail" || action.endsWith("_failed")) {
    const code = result && result !== "failed" ? result : action;
    const m = mapCode(code);
    const loginTitle = code === "LOGIN_CONFIRM_REQUIRED" ? ({
      UIA_STADIUM_ENTRY_NOT_FOUND: "首页未找到场馆预约入口",
      UIA_HOME_SCROLL_UNAVAILABLE: "首页无法自动滚动查找入口",
      UIA_SCROLL_UNAVAILABLE: "登录或授权页无法自动滚动",
      UIA_LOGIN_CONTROL_NOT_FOUND: "未找到可操作的立即登录",
      UIA_AUTH_CONTROL_NOT_FOUND: "未找到可操作的协议或授权控件",
      UIA_SCROLL_LIMIT_REACHED: "滚动查找达到步数上限",
      UIA_SEARCH_TIMEOUT: "登录或场馆入口查找超时",
      UIA_PORTRAIT_UNAVAILABLE: "小程序无法调整为竖屏",
      UIA_WINDOW_LAYOUT_UNAVAILABLE: "小程序窗口尺寸调整失败",
      UIA_WINDOW_LAYOUT_UNSTABLE: "小程序窗口尺寸持续回弹",
      LOGIN_FOCUS_REQUIRED: "小程序未能保持在前台",
      UIA_PIN_UNAVAILABLE: "小程序置顶准备失败",
      UIA_STADIUM_NAVIGATION_TIMEOUT: "场馆预约页面未打开",
      MINIPROGRAM_NOT_VISIBLE: "厦大体育小程序未显示",
      UIA_TARGET_MISSING: "厦大体育小程序未显示",
      H5_ENTRY_STILL_EXPIRED: "预约登录凭证仍已失效",
      H5_ENTRY_UNAVAILABLE: "预约登录凭证未能读取",
      UIA_LOGIN_UNAVAILABLE: "无法识别小程序当前页面",
      UIA_NAVIGATION_TIMEOUT: "登录或授权页面未跳转",
      UIA_INPUT_OUTCOME_UNKNOWN: "小程序操作结果未确认",
    } as Record<string,string>)[ev.http?.reason ?? ""] : undefined;
    if (["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "ORDER_RECHECK_UNCONFIRMED"].includes(code)) {
      return { kind: "info", code, title: m.title, detail: ev.http?.message || m.detail,
        pageId: ev.page_id ?? null, ts: ev.ts ?? null };
    }
    return {
      kind: "error",
      code,
      title: loginTitle || (m.kind === "error" ? m.title : `失败：${code}`),
      detail: ev.http?.message || m.detail,
      pageId: ev.page_id ?? null,
      ts: ev.ts ?? null,
    };
  }

  if (action === "cancelled") {
    const m = mapCode("cancelled");
    return {
      kind: "info",
      code: "cancelled",
      title: m.title,
      detail: m.detail,
      pageId: ev.page_id ?? null,
      ts: ev.ts ?? null,
    };
  }

  return null;
}

export function outcomeKey(o: RunOutcome): string {
  return `${o.ts ?? ""}|${o.code}|${o.kind}`;
}

/** Toast 文案：短 title，body 可选补 detail */
export function toastPayloadFromOutcome(o: RunOutcome): {
  kind: "success" | "info" | "warn" | "error";
  title: string;
  body?: string;
} {
  return {
    kind: o.kind,
    title: o.title,
    body: o.detail !== o.title ? o.detail : undefined,
  };
}
