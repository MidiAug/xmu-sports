import type { RunEvent } from "../types";

export type RunFailure = {
  category: string; explanation: string; submission: string;
  title?: string; summary?: string;
  phase: string; api?: string; method?: string; status?: number; elapsedMs?: number;
  errorType?: string; request?: Record<string, unknown>;
  timeout?: {connect: number; read: number}; serverMessage?: string;
};

const APIS: Record<string, string> = {
  login: "核验体育登录", order_list: "查询预约列表", order_detail: "查询订单详情",
  venue_list: "查询场馆", venue_detail: "查询场馆详情", intervals: "查询实时场次",
  eligibility: "预约条件校验", venue_config: "查询预约配置", captcha_image: "获取验证码", submit: "提交预约",
};
const PHASES: Record<string, string> = {
  authenticating: "核验体育登录", checking_orders: "检查已有预约", resolving_venue: "匹配运动与场馆",
  querying_slots: "查询实时场次", checking_slot: "选择场次", server_precheck: "校验预约条件",
  checking_config: "检查预约配置", captcha: "获取验证码", submitting: "提交预约",
  order_accepted: "学校已接收提交", verifying_order: "查单确认", order_unconfirmed: "等待核对提交结果",
  reading_entry: "读取登录入口", local_eligibility: "本地预约条件检查",
};
const UNCERTAIN = new Set(["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "ORDER_RECHECK_UNCONFIRMED"]);
const BEFORE_SEND = new Set(["authenticating", "reading_entry", "local_eligibility", "checking_orders", "resolving_venue", "querying_slots", "checking_slot", "server_precheck", "checking_config", "captcha"]);

export function diagnoseRunFailure(event: RunEvent, events: RunEvent[] = []): RunFailure {
  const code = event.result ?? "";
  const rows = events.filter(row => row.task_id === event.task_id && row.run_id === event.run_id);
  const exchanges = rows.filter(row => row.action === "http_exchange");
  const lastExchange = event.http?.failure_exchange ?? exchanges.at(-1)?.http;
  const phase = event.http?.phase ?? lastExchange?.phase ?? "";
  // A successful exchange from an earlier phase is not evidence of this failure.
  const exchange = lastExchange && (lastExchange.error || (lastExchange.http_status ?? 0) >= 400 || lastExchange.phase === phase) ? lastExchange : undefined;
  const error = exchange?.error_type ?? exchange?.error;
  const status = exchange?.http_status;
  const api = exchange?.api ? APIS[exchange.api] ?? exchange.api : undefined;
  const sent = rows.some(row => ["submitting", "order_accepted", "verifying_order"].includes(row.http?.phase ?? row.result ?? ""))
    || (event.http?.diagnostics?.http?.submit?.count ?? 0) > 0;
  const noSend = !sent && (event.http?.execution_proof?.add_order_calls === 0
    || event.http?.captcha_requested === false && BEFORE_SEND.has(phase));
  const submission = UNCERTAIN.has(code) || sent ? "已进入提交流程，需以查单结果为准" : noSend ? "未发送预约提交" : "本条记录不足以确认提交状态";
  let category = "其他执行错误", explanation = "保留原错误码和过程记录；当前证据不足以确定更具体的原因。";
  let title: string | undefined;
  let summary: string | undefined;
  if (UNCERTAIN.has(code)) {
    category = "结果待确认"; explanation = "提交是否成功尚未确认，保留原查单与去重记录，不自动重复下单。";
  } else if (["HTTP_FAILED", "RATE_LIMITED", "SERVER_BUSY", "CAPTCHA_FETCH_FAILED", "INVALID_RESPONSE", "UNEXPECTED_REDIRECT"].includes(code)) {
    if (code === "RATE_LIMITED" || status === 429) {
      category = "明确限流"; title = "学校接口限制请求频率";
      explanation = "学校返回了明确的限流错误；本次已停止，不自动重发预约。";
    } else if (code === "SERVER_BUSY" || status !== undefined && status >= 500) {
      category = "学校服务异常"; title = "学校服务暂时异常";
      explanation = "学校接口返回服务端错误，不能据此推断本地请求参数有误。";
    } else if (["timeout", "connect_timeout", "read_timeout"].includes(error ?? "")) {
      category = error === "connect_timeout" ? "连接超时" : error === "read_timeout" ? "读取超时" : "请求超时";
      title = `${exchange?.api === "order_list" || exchange?.api === "order_detail" ? "订单查询" : "学校接口"}${error === "timeout" ? "超时" : category}`;
      explanation = error === "connect_timeout" ? "在配置的连接时限内未建立连接；可能与网络链路或学校接入服务有关，尚无参数错误证据。"
        : error === "read_timeout" ? "请求等待学校响应超过读取时限；本地日志不能单独区分学校处理慢与链路延迟。"
        : "日志只记录了timeout，未区分连接和读取超时；不能确定是学校响应慢还是本地网络，也没有明确限流或参数错误证据。";
    } else if (["connection", "proxy", "tls"].includes(error ?? "")) {
      category = error === "proxy" ? "代理连接异常" : error === "tls" ? "安全连接异常" : "网络连接异常";
      title = category;
      explanation = error === "proxy" ? "请求在代理连接环节失败，学校是否收到请求尚不明确。"
        : error === "tls" ? "TLS安全连接建立或校验失败；当前证据不表示学校拒绝了预约参数。"
        : "连接建立或传输失败；现有记录未进一步区分DNS、断线等原因，不直接归类为限流。";
    } else if (error === "invalid_json" || code === "INVALID_RESPONSE") {
      category = "响应格式异常"; title = "学校响应格式异常";
      explanation = "学校返回的内容无法按预期解析；可能是错误页或接口格式变化，需要结合HTTP状态和学校说明判断。";
    } else if (status !== undefined && status >= 400) {
      category = "HTTP请求被拒绝"; title = `学校接口返回 HTTP ${status}`;
      explanation = "学校返回HTTP拒绝响应；这与参数错误、权限限制是否有关，要以学校返回内容为准。";
    } else if (code === "UNEXPECTED_REDIRECT") {
      category = "接口重定向"; explanation = "接口发生重定向，程序没有继续向跳转地址发送会话。";
    } else {
      category = "接口请求失败"; explanation = "旧记录没有保留可用于细分原因的传输异常或HTTP响应，不能归因于限流、参数或某一网络环节。";
    }
    if (title) summary = `${noSend ? "本次未提交预约。" : sent ? "已进入提交流程，请先核对订单。" : ""}${explanation}`;
  } else if (/SESSION|LOGIN|ENTRY|AUTH|ACCOUNT_CHANGED|ACCOUNT_MISMATCH|PROFILE/.test(code)) {
    category = "登录或账号异常"; explanation = "按原错误码处理登录入口、会话或账号核验，不将普通网络失败当作登录过期。";
  } else if (/CAPTCHA/.test(code)) {
    category = "验证码异常"; explanation = "验证码获取、识别或学校校验未通过，具体原因以原错误码和学校说明为准。";
  } else if (/^HTTP_WORKER|HTTP_COMPONENT|^RUNTIME|^MISSING_/.test(code)) {
    category = "本地组件异常"; explanation = "本地执行组件或运行环境未完成工作，不能归因于学校限流。";
  } else if (code === "API_REJECTED") {
    category = "学校业务拒绝"; explanation = "学校返回了业务拒绝；具体原因使用学校原说明，不凭HTTP 200判断预约成功。";
  } else if (/SLOT|VENUE|DATE|BOOKING_CLOSED|NOT_OPEN|ACCOUNT_LIMIT|SPORT_DAY|FALLBACK|REMARK|PHONE/.test(code)) {
    category = "预约条件不满足"; explanation = "日期、场次、名额或账号预约条件未满足，以本次错误码和学校说明为准。";
  }
  const response = exchange?.response;
  const serverMessage = response && typeof response === "object" && "info" in response && typeof response.info === "string" ? response.info : undefined;
  return {category, explanation, title, summary, submission, phase: (PHASES[phase] ?? phase) || "未记录", api, method: exchange?.method,
    status, elapsedMs: exchange?.elapsed_ms, errorType: error, request: exchange?.request, timeout: exchange?.timeout_seconds, serverMessage};
}
