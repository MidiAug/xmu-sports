import type { RunEvent } from "../types";

const PHASES: Record<string, [string, string]> = {
  pre_submit_auth_recovering: ["auth", "正式执行时登录再次失效；已确认尚未提交，恢复原账号登录"],
  pre_submit_auth_restored: ["auth", "原账号已重新核验，沿用原任务继续一次，保留原请求去重标识"],
  worker_model_warming: ["auth", "提前启动请求进程并加载本地验证码模型，不提交预约"],
  worker_warmed: ["auth", "请求进程与验证码模型已预热，等待开抢"],
  worker_warm_rechecked: ["auth", "已确认预热进程仍可用，沿用已加载的验证码模型"],
  semantic_login_control: ["auth", "识别当前登录页面和可用操作"],
  normal_ui_reenter: ["auth", "重新打开厦大体育，检查从小程序到场馆预约的登录流程"],
  normal_ui_refreshing: ["auth", "页面暂未提供可操作入口，保存现场后重新识别并继续等待"],
  uia_select: ["auth", "已勾选并确认登录协议"],
  uia_toggle: ["auth", "已确认登录协议处于勾选状态"],
  uia_invoke: ["auth", "已执行页面按钮操作"],
  uia_default_action: ["auth", "已执行页面元素操作"],
  uia_covered_window_message: ["auth", "在被遮挡窗口内操作页面，未移动光标"],
  uia_window_message: ["auth", "通过窗口消息操作页面，未移动光标"],
  normal_ui_background: ["auth", "在后台窗口恢复登录，不移动光标"],
  normal_ui_foreground: ["auth", "已将厦大体育置于前台，操作前确认窗口焦点，登录后自动关闭"],
  normal_ui_closed: ["auth", "小程序已关闭，登录凭证核验通过"],
  normal_ui_close_pending: ["auth", "登录已完成，小程序窗口尚未关闭"],
  uia_element_mouse: ["auth", "已点击识别到的页面元素"],
  visual_login_fallback: ["auth", "部分页面未提供可操作元素，切换到界面识别"],
  normal_ui_opening: ["auth", "打开厦大体育，通过正常界面恢复登录"],
  normal_ui_http_verifying: ["auth", "已进入场馆，用 HTTP 核验账号并读取已有预约"],
  normal_ui_http_ready: ["auth", "正常界面登录已交给 HTTP，账号与已有预约读取通过"],
  normal_ui_manual_required: ["auth", "请在厦大体育完成协议、授权或页面确认；完成后自动继续，超时可重新检测"],
  h5_entry_reused: ["auth", "已有场馆入口有效，账号核验通过，继续使用"],
  scheduled_h5_ready: ["auth", "H5 入口已提前就绪，等待临近开抢时校验"],
  scheduled_h5_rechecking: ["auth", "新入口准备中断，重新验证已有 H5 与任务账号"],
  scheduled_h5_reused: ["auth", "已有 H5 与任务账号重新验证通过，继续定时预约"],
  scheduled_waiting_fire: ["auth", "登录与 H5 已就绪，等待设定时间开始预约"],
  local_eligibility: ["auth", "检查日期、开放时间与时段"],
  reading_entry: ["auth", "读取本机体育登录票据"],
  authenticating: ["auth", "验证登录并建立预约会话"],
  runtime_connecting: ["auth", "连接厦大体育的代码操作通道"],
  runtime_reconnecting: ["auth", "小程序连接正在切换，等待重新连接后继续"],
  runtime_ready: ["auth", "微信内核兼容性检查通过，自动入口通道已连接"],
  launching_runtime: ["auth", "启动微信小程序内核，准备自动入口"],
  runtime_reenter: ["auth", "重新进入小程序，检查体育登录状态"],
  native_login_opening: ["auth", "进入体育登录页面"],
  native_agreement: ["auth", "确认体育登录协议"],
  native_authorizing: ["auth", "执行体育授权登录"],
  native_login_pending: ["auth", "等待体育登录完成"],
  waiting_miniprogram_focus: ["auth", "等待小程序获得操作焦点；请暂时退出全屏或切换到小程序"],
  entering_stadium: ["auth", "进入场馆预约，获取新的 H5 入口"],
  returning_home: ["auth", "返回体育首页"],
  waiting_native_page: ["auth", "等待小程序页面加载"],
  waiting_h5_entry: ["auth", "等待新 H5 入口写入本机缓存"],
  fresh_entry_ready: ["auth", "新 H5 入口与本人账号验证通过"],
  closing_miniprogram: ["auth", "关闭小程序，保留电脑微信运行"],
  launching_login: ["auth", "通过微信协议唤起厦大体育"],
  waiting_login_entry: ["auth", "等待新票据；请在小程序确认登录并进入场馆预约"],
  validating_login_entry: ["auth", "发现新票据，验证会话与本人账号"],
  login_entry_rejected: ["auth", "新入口仍被服务器拒绝，等待登录确认"],
  checking_orders: ["query", "检查已有预约，避免重复下单"],
  resolving_venue: ["query", "匹配运动与场馆"],
  querying_slots: ["query", "查询目标日期的实时场次"],
  waiting_slot_retry: ["query", "场次查询返回参数错误，等待 2 秒后重试一次"],
  selecting_candidate: ["query", "选择候选时段"],
  discovering_fallbacks: ["query", "首选场次不可约，查询备选时段"],
  candidate_rejected: ["verify", "候选场次未通过预约检查"],
  waiting_open: ["query", "等待学校发布场次（最多 5 秒）"],
  checking_slot: ["verify", "检查场次余量与锁定状态"],
  server_precheck: ["verify", "请求学校校验预约资格"],
  checking_config: ["verify", "读取预约规则与确认信息"],
  slot_ready: ["verify", "场次与预约资格校验通过"],
  warming: ["captcha", "准备本地验证码识别组件"],
  captcha: ["captcha", "获取并识别验证码"],
  captcha_recognized: ["captcha", "验证码识别完成，准备提交"],
  captcha_retry: ["captcha", "验证码未通过，换图后重新校验（合计最多 3 次）"],
  submitting: ["submit", "发送预约请求，等待学校响应"],
  order_accepted: ["submit", "学校已接受预约，继续查单确认"],
  verifying_order: ["order", "查询我的预约，确认匹配订单"],
  submission_uncertain: ["order", "提交响应未确认，开始自动查单，不重复提交"],
  waiting_order_sync: ["order", "等待学校同步预约结果后再查单"],
  rechecking_orders: ["order", "核对我的预约列表与详情，匹配当次实际提交的场地和时段"],
  order_check_retry: ["order", "暂未确认匹配预约，稍后再次只读查询"],
  order_reconciled: ["order", "已查到匹配预约，确认预约成功"],
  order_unconfirmed: ["order", "自动查单仍未确认，保留待确认状态，不重复提交"],
};

export const BOOKING_STEPS = [
  ["auth", "登录检测"], ["captcha", "验证码"], ["query", "实时场次"],
  ["verify", "预约校验"], ["submit", "提交预约"], ["order", "查单确认"],
] as const;

export function progressStep(event: RunEvent): string {
  if (event.action === "done") return "order";
  return PHASES[event.http?.phase ?? event.result ?? ""]?.[0] ?? "auth";
}

export function progressMessage(event: RunEvent): string {
  if (event.action === "done" && event.result === "http_auth_ready") return "体育登录已就绪，未提交预约";
  if (event.action === "done" && event.result === "http_prepared") return "预约预检通过，未提交订单";
  if (event.action === "done" && event.result === "http_order_rechecked") return "再次查单已确认原预约，未重复下单";
  if (event.action === "done") return event.result === "http_already_reserved"
    ? "查到已有预约，本次未重复提交" : event.http?.verified
    ? "预约成功，已查到匹配订单" : event.http?.message ?? "流程已完成";
  if (event.action === "fail") return event.http?.message ?? (["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED"].includes(event.result ?? "")
    ? "预约结果待确认，未重复提交" : `预约停止：${event.result ?? "未知错误"}`);
  if (event.action === "cancelled") return "本次预约流程已取消";
  if (event.action === "spawn") return event.result === "recheck_started" ? "开始只读核对原预约，不提交订单" : "开始执行预约任务";
  if (event.action === "auth_check") return event.result === "ready" ? "登录检测通过" : event.http?.message ?? "登录检测未通过";
  if (event.action === "login_recovery") return "打开体育小程序，准备新的场馆预约入口";
  const phase = event.http?.phase ?? event.result ?? "";
  let message = PHASES[phase]?.[1] ?? `流程状态：${phase || event.action || "未提供说明"}`;
  if (["waiting_order_sync", "rechecking_orders", "order_check_retry", "order_reconciled"].includes(phase)
      && event.http?.check_attempt !== undefined && event.http?.check_total !== undefined)
    message += `（第 ${event.http.check_attempt}/${event.http.check_total} 轮）`;
  if (phase === "waiting_order_sync" && event.http?.retry_after_seconds !== undefined)
    message += `　${event.http.retry_after_seconds} 秒后查询`;
  if (["waiting_login_entry", "waiting_miniprogram_focus"].includes(phase) && event.http?.remaining_seconds !== undefined)
    message += `（剩余 ${event.http.remaining_seconds} 秒）`;
  if (phase === "candidate_rejected") message += `：${event.http?.message ?? "不可预约"}`;
  if (phase === "slot_ready" && event.http?.court) message += `　${event.http.court} 号场地`;
  if (phase === "selecting_candidate" && event.http?.candidate_index !== undefined)
    message += `（第 ${event.http.candidate_index + 1} 个候选）`;
  const target = event.http?.target;
  if (target && ["selecting_candidate", "checking_slot", "submitting"].includes(phase))
    message += `　${target.date ?? ""} ${target.time ?? ""}–${target.end ?? ""}　${target.court ? `${target.court} 号场地` : "自动选择场地"}`;
  return message;
}

export function currentRunEvents(events: RunEvent[], last: RunEvent | null): RunEvent[] {
  if (!last) return [];
  const run = last.run_id;
  return events.filter(event => event.task_id === last.task_id && event.run_id === run);
}

export function elapsedLabel(events: RunEvent[], running: boolean, now = Date.now()): string {
  if (!events.length) return "";
  const first = Date.parse(events[0].ts);
  const end = running ? now : Date.parse(events[events.length - 1].ts);
  return `${(Math.max(0, end - first) / 1000).toFixed(1)} 秒`;
}
