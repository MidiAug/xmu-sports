import { SVG } from "../ui/icons";
import { renderPlanQueue } from "../ui/plan-queue";
import { bookingPlans, type PlansSnapshot } from "../lib/booking-plans-store";
import { invoke } from "@tauri-apps/api/core";
import type { LiveStatus, RunEvent, WechatProbe } from "../types";
import { ingestLiveStatus } from "../lib/run-feedback";
import { outcomeFromEvent } from "../lib/run-outcome";
import { clearLiveTick, setRunIndicator } from "../lib/run-indicator";
import {
  formatTargetBrief,
  formatTaskSportVenue,
} from "../lib/format-human";
import { BOOKING_STEPS, currentRunEvents, elapsedLabel, progressMessage, progressStep } from "../lib/booking-progress";
import { log } from "../lib/log";
import { toastError, toastSuccess } from "../ui/toast";
import { mountPage, type PageScope } from "../lib/page-lifecycle";
import { patchHtml } from "../ui/patch-html";
import { mountOrderRecheck } from "../ui/order-recheck";
import {
  bumpLiveRenderGen,
  liveRenderGen,
  liveStatusCache,
  liveWxCache,
  setLiveTickTimer,
  setLiveWxCache,
  tab,
} from "../state";

export type RenderLiveOpts = {
  /** 只刷新状态，不探测微信（武装变更等） */
  statusOnly?: boolean;
  status?: LiveStatus;
};

function escText(s: string): string {
  return s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function outcomeBarHtml(ev: RunEvent | null): string {
  const o = outcomeFromEvent(ev);
  if (!o) return "";
  const cls =
    o.kind === "error" ? "err" : o.kind === "success" ? "ok" : "info";
  return `
    <div class="live-outcome ${cls}" role="status" aria-live="polite">
      <p class="live-outcome-title">${escText(o.title)}</p>
      <p class="live-outcome-detail">${escText(o.detail)}</p>
    </div>`;
}

function executionHtml(st: LiveStatus | null, detailsOpen = false): string {
  const last = st?.last_event ?? null;
  const events = currentRunEvents(st?.recent_events ?? (last ? [last] : []), last);
  if (!events.length || (!events.some(event => event.http || event.result?.startsWith("http_")) && !(st?.running && (st.task?.executor === "http" || st.operation === "recheck")))) return "";
  const running = !!st?.running;
  const authOnly = st?.operation === "auth" || last?.result === "http_auth_ready" || events.some(event => event.result === "auth_check_started");
  const prepareOnly = st?.operation === "prepare" || last?.result === "http_prepared" || events.some(event => event.result === "prepare_started");
  const recheckOnly = st?.operation === "recheck" || events.some(event => event.result === "recheck_started");
  const phaseEvents = events.filter(event => event.action === "http_phase");
  const latestPhase = phaseEvents.at(-1);
  const uncertain = ["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "http_submitted", "ORDER_RECHECK_UNCONFIRMED"].includes(last?.result ?? "");
  const failed = !uncertain && (last?.action === "fail" || last?.action === "cancelled");
  const active = failed && last?.http?.phase ? progressStep(last) : latestPhase ? progressStep(latestPhase) : "auth";
  const observed = new Set(phaseEvents.map(progressStep));
  if (last?.action === "done" && !["http_prepared", "http_auth_ready"].includes(last.result ?? "")) observed.add("order");
  const steps = (recheckOnly ? BOOKING_STEPS.filter(([id]) => ["auth", "order"].includes(id)) : authOnly ? BOOKING_STEPS.slice(0, 1) : BOOKING_STEPS).map(([id, label],stepIndex) => {
    const current = running && active === id;
    const error = failed && active === id;
    const pending = !running && uncertain && id === "order";
    const seen = observed.has(id) && !pending;
    return `<li class="${error ? "fail" : current ? "current" : pending ? "pending" : seen ? "done" : ""}" ${current ? 'aria-current="step"' : ""}><span class="booking-step-icon">${error ? "!" : pending ? "?" : seen && !current ? "✓" : recheckOnly ? stepIndex+1 : BOOKING_STEPS.findIndex(step => step[0] === id) + 1}</span><span>${label}</span></li>`;
  }).join("");
  const rows = events.filter(event => ["spawn", "http_phase", "auth_check", "login_recovery", "done", "fail", "cancelled"].includes(event.action ?? ""));
  const timeline = rows.map((event, index) => {
    const next = events[events.indexOf(event) + 1];
    const duration = next ? Math.max(0, Date.parse(next.ts) - Date.parse(event.ts)) / 1000 : null;
    const time = new Date(event.ts).toLocaleTimeString("zh-CN", {hour12: false, timeZone: "Asia/Shanghai"});
    const rowState = index === rows.length - 1 && running ? "active" : event.action === "fail" ? "failed" : event.action === "done" ? "complete" : "";
    return `<li class="${rowState}"><div class="booking-event-meta"><time datetime="${escText(event.ts)}">${escText(time)}</time>${duration !== null ? `<small>${duration.toFixed(1)}s</small>` : ""}</div><span>${escText(progressMessage(event))}</span></li>`;
  }).join("");
  const result = last?.http;
  const selected = [...events].reverse().find(event => event.http?.court)?.http?.court;
  // Results belong to the run's frozen goal, even after editing the next task.
  const target = result?.target ?? [...events].reverse().find(event => event.http?.target)?.http?.target;
  const venueLabel = target?.sport && target?.venue ? `${target.venue}　${target.sport}`
    : running && st?.task ? formatTaskSportVenue(st.task) : "本次预约目标";
  const date = result?.date ?? target?.date ?? (!recheckOnly && running ? st?.task?.target_date : "") ?? "";
  const slot = result?.time ?? (target ? `${target.time ?? ""}–${target.end ?? ""}` : !recheckOnly && running ? `${st?.task?.target_time_start ?? ""}–${st?.task?.target_time_end ?? ""}` : "");
  const outcome = last ? outcomeFromEvent(last) : null;
  const state = running ? "running" : uncertain ? "unconfirmed" : failed ? "failed" : (result?.verified || last?.result === "http_auth_ready") ? "success" : "finished";
  const confirming = running && latestPhase && progressStep(latestPhase) === "order";
  const title = running ? recheckOnly ? "正在再次查单" : confirming ? "正在确认预约结果" : st?.operation === "scheduled" && st.armed ? "定时预约准备中" : authOnly ? "正在检查体育登录" : prepareOnly ? "正在检查预约条件" : "预约进行中" : outcome?.title ?? "预约流程已结束";
  const detail = running ? (last ? progressMessage(last) : "正在启动…") : outcome?.detail ?? (last ? progressMessage(last) : "");
  const court = result?.court ?? selected ?? target?.court;
  return `<section class="booking-execution ${state}${authOnly ? " booking-auth" : ""}${running ? "" : " booking-recent"}" aria-label="预约执行状态">
    <div class="booking-execution-head"><span class="booking-state-icon" aria-hidden="true">${running ? '<span class="booking-spinner"></span>' : state === "success" ? "✓" : failed ? "!" : "·"}</span><div class="booking-head-copy"><span class="booking-eyebrow">${authOnly ? "体育登录" : prepareOnly ? "预约条件检查" : recheckOnly ? running ? "只读查单" : "最近一次查单" : running ? "正在执行" : "最近一次预约"}</span><h2 role="status">${escText(title)}</h2></div><span class="booking-elapsed">${running ? "已用时" : "总用时"}<strong>${elapsedLabel(events, running)}</strong></span>${running ? '<button type="button" class="btn ghost booking-stop" id="disarm_hang">停止</button>' : ""}</div>
    <p class="booking-current-detail">${escText(detail)}</p>${confirming ? '<p class="booking-current-detail">正在只读查单；停止只会结束后续检查，已经发送的预约不会被撤回。</p>' : ""}
    ${!authOnly && (target || running && st?.task) ? `<div class="booking-target"><strong>${escText(venueLabel)}</strong><div><span>${escText(date)}</span><span>${escText(slot)}</span><span>${escText(court ? `${court} 号场地` : "自动选择场地")}</span></div>${result?.order_id ? `<small>订单号 ${escText(String(result.order_id))}</small>` : ""}</div>` : ""}
    ${!authOnly ? `<ol class="booking-track${recheckOnly ? " recheck-only" : authOnly ? " auth-only" : ""}" aria-label="执行步骤">${steps}</ol>` : ""}
    <details class="booking-details" ${detailsOpen ? "open" : ""}><summary><span class="booking-details-label">${SVG.tabHistory}<span>过程记录</span><span class="booking-event-count">${rows.length} 条</span></span><span class="booking-chevron">${SVG.chevron}</span></summary><div class="booking-details-content"><ol class="live-timeline" aria-label="预约过程记录" tabindex="0">${timeline}</ol><p class="booking-run-id">${escText(`执行编号 ${last?.run_id ?? ""}`)}</p></div></details>
  </section>`;
}

function recoveryHtml(event: RunEvent | null): string {
  const code = event?.result ?? "";
  if (["MINIPROGRAM_NOT_VISIBLE","UIA_TARGET_MISSING","MINIPROGRAM_LAUNCH_FAILED"].includes(event?.http?.reason ?? code)) return "";
  if (code.startsWith("RUNTIME_") && code !== "RUNTIME_CLOSE_FAILED") return '<div class="task-busy-note"><span>请重启电脑微信后重试；仍失败请导出诊断日志。</span><button type="button" class="btn ghost" data-tab="settings">前往设置</button></div>';
  const login = ["LOGIN_CONFIRM_REQUIRED", "SESSION_EXPIRED", "NO_ENTRY", "RUNTIME_UNAVAILABLE", "RUNTIME_UNSUPPORTED", "LOGIN_FOCUS_REQUIRED"].includes(code);
  const uncertain = ["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "http_submitted", "ORDER_RECHECK_UNCONFIRMED"].includes(code);
  if (login || uncertain) return `<div class="task-busy-note"><span>${uncertain ? "可在待确认记录点击「再次查单」，或在小程序「我的预约」核对；不会自动重发。" : "完成体育登录并进入场馆预约后，可以重新执行原任务。"}</span><button type="button" class="btn ghost" id="open_booking">打开小程序${uncertain ? "确认" : "登录"}</button></div>`;
  if (event?.action === "fail") return '<div class="task-busy-note"><span>可调整日期、场馆或备选策略，再尝试预约。</span><button type="button" class="btn ghost" data-tab="setup">修改预约目标</button></div>';
  return "";
}

function emptyProgressHtml(): string {
  return `<div class="activity-empty"><span class="activity-empty-icon" aria-hidden="true">${SVG.tabHistory}</span><p class="live-brief-title">尚未配置任务</p><p class="live-brief-sub">添加预约后，在这里查看执行进度</p><p class="live-brief-cta"><button type="button" class="btn primary" data-tab="setup">去配置任务</button></p></div>`;
}

function envRowHtml(wxText: string, wxRunning: boolean): string {
  return `
    <div class="live-env">
      <span class="live-env-value">
        <span class="dot ${wxRunning ? "on" : ""}" id="wx_dot"></span>
        <span id="wx_text">微信（${wxText}）</span>
      </span>
    </div>`;
}

function paintPlanContext(panel: Element, status: LiveStatus | null, snapshot: PlansSnapshot) {
  const host = panel.querySelector<HTMLElement>("#live_plan_queue");
  if (!host) return;
  const scheduled = renderPlanQueue(host, snapshot.view, status);
  if (snapshot.error) {
    const error = document.createElement("p"); error.className = "msg err";
    error.textContent = `计划暂未读取：${String(snapshot.error)}`; host.append(error);
  }
  const results = panel.querySelector<HTMLElement>("#live_plan_results");
  const labels: Record<string,string> = {succeeded:"已约到",existing:"已有预约",failed:"未约到"};
  const attempts = snapshot.view?.plans.attempts.filter(a => labels[a.status] && a.task.task_id !== status?.last_event?.task_id).slice(-3).reverse() ?? [];
  const hasExecution = Boolean(panel.querySelector("#live_execution")?.childElementCount);
  if(results) patchHtml(results, !status?.running && !hasExecution && attempts.length ? `<section class="live-results"><header><h3>最近预约结果</h3><button class="btn text" data-tab="history">查看记录 →</button></header>${attempts.map(a=>`<article><div><strong>${escText(a.task.sport)}</strong><p>${escText(`${a.task.venue} · ${formatTargetBrief(a.task)}`)}</p></div><span data-result="${a.status}">${labels[a.status]}</span></article>`).join("")}</section>` : "");
  const empty = panel.querySelector<HTMLElement>(".live-upcoming-empty");
  if (empty) empty.hidden = scheduled || !!status?.running;
  const draft = panel.querySelector<HTMLElement>("#live_single_context");
  if (draft) draft.hidden = !!status?.armed || !!status?.running || !!status?.task?.plan_id || !status?.task;
}

function paintLiveBody(
  panel: Element,
  st: LiveStatus | null,
  wx: WechatProbe | null,
  opts?: { knownStatus?: boolean; knownWx?: boolean; scope?: PageScope },
) {
  const previousTimeline = panel.querySelector<HTMLElement>(".live-timeline");
  const previousQueue = panel.querySelector("#live_plan_queue")?.innerHTML ?? "";
  const previousPending = panel.querySelector("#pending_order_checks")?.innerHTML ?? "";
  const auxiliaryOpen = panel.querySelector<HTMLDetailsElement>(".live-auxiliary")?.open ?? false;
  const detailsOpen = panel.querySelector<HTMLDetailsElement>(".booking-details")?.open ?? false;
  const timelineTop = previousTimeline?.scrollTop ?? 0;
  const followTimeline = !previousTimeline || previousTimeline.scrollHeight - previousTimeline.clientHeight - timelineTop < 32;
  clearLiveTick();
  const knownStatus = opts?.knownStatus ?? st !== null;
  const knownWx = opts?.knownWx ?? wx !== null;
  const t = st?.task ?? null;
  const ev = st?.last_event ?? null;
  const armed = Boolean(st?.armed);
  const running = Boolean(st?.running);
  const stopping = Boolean(st?.stopping);
  const wxRunning = Boolean(wx?.running);
  const busy = armed || running;

  if (knownStatus) {
    setRunIndicator(busy);
  }

  const wxText = !knownWx ? "检测中…" : wxRunning ? "已运行" : "未运行";
  const executionBlock = executionHtml(st, detailsOpen);
  const outcomeBlock = !running && !executionBlock ? outcomeBarHtml(ev) : "";

  const auxiliary = !running && Boolean(ev?.action === "done" && (ev.result === "http_auth_ready" || ev.result === "http_prepared" || st?.operation === "auth" || st?.operation === "prepare"));
  const resultBlock = auxiliary
    ? `<details class="live-auxiliary" ${auxiliaryOpen ? "open" : ""}><summary><span>最近检查</span><span>${escText(outcomeFromEvent(ev)?.title ?? "体育登录检查")} ${"›"}</span></summary>${executionBlock || outcomeBlock}</details>`
    : executionBlock || outcomeBlock;
  const mainBlock = t ? `<div class="live-draft-heading"><div><span class="section-eyebrow">单次预约 · 未启动</span><p>${escText(t.sport)} · ${escText(t.venue)}<br>${escText(formatTargetBrief(t))}</p></div><button type="button" class="btn ghost live-context-action" id="live_draft_edit" data-tab="setup">修改预约${SVG.arrowUpRight}</button></div>` : "";

  patchHtml(panel, `
    <div class="page-stack live-page" data-idle="${!busy && !t && !executionBlock && !outcomeBlock}">
      <div class="page-heading"><div><h2>预约进度</h2><p>下一次开抢与最近预约结果</p></div><div class="live-heading-actions"><button type="button" class="btn ghost" id="live_configuration" ${t && running ? "" : "hidden"} data-tab="${t?.plan_id ? "plans" : "setup"}" ${t?.plan_id ? `data-plan-id="${escText(t.plan_id)}"` : ""}>${t?.plan_id ? "查看本项计划" : "查看配置"}</button>${envRowHtml(wxText, wxRunning)}</div></div>
      <div class="live-board"><div class="live-primary"><section id="live_plan_queue" class="live-plan-queue">${previousQueue}</section><div class="live-upcoming-empty" hidden>${t ? '<div class="activity-empty"><p class="live-brief-title">没有待执行预约</p><p class="live-brief-sub">启动单次定时或启用计划后，在这里查看开抢安排</p><p class="live-brief-cta"><button class="btn primary" data-tab="setup">去预约</button></p></div>' : emptyProgressHtml()}</div><div id="live_single_context" hidden>${mainBlock}</div></div><aside class="live-secondary"><div class="live-workspace"><div id="live_execution">${!auxiliary ? resultBlock : ""}</div><div id="live_plan_results"></div><div id="live_auxiliary">${auxiliary ? resultBlock : ""}</div><div id="pending_order_checks">${previousPending}</div>
      ${!running ? recoveryHtml(ev) : ""}
      </div></aside></div>
    </div>
    ${stopping ? '<p class="task-busy-note" role="status">正在停止，请等待已发送请求的结果确认。</p>' : ""}
  `);
  paintPlanContext(panel, st, bookingPlans.snapshot());

  const pendingHost=panel.querySelector<HTMLElement>("#pending_order_checks");
  if (pendingHost) {
    if (!running) void mountOrderRecheck(pendingHost,busy,()=>!opts?.scope || opts.scope.current());
    else pendingHost.querySelectorAll<HTMLButtonElement>("[data-order-recheck]").forEach(button => { button.disabled = true; });
  }
  if (stopping) {
    const stop = panel.querySelector<HTMLButtonElement>("#disarm_hang");
    if (stop) { stop.disabled = true; stop.textContent = "正在停止…"; }
  }
  const timeline = panel.querySelector<HTMLElement>(".live-timeline");
  if (timeline) timeline.scrollTop = followTimeline ? timeline.scrollHeight : timelineTop;
  const recovery = panel.querySelector<HTMLButtonElement>("#open_booking");
  if (recovery) recovery.onclick = async () => {
    recovery.disabled = true;
    try { await invoke("launch_xmu_miniprogram"); toastSuccess("已请求打开体育小程序", "请在小程序完成登录或查看我的预约"); }
    catch (error) { toastError("未能打开小程序", String(error)); }
    finally { if (recovery.isConnected) recovery.disabled = false; }
  };

  if (running) {
    let tick = 0;
    setLiveTickTimer(
      window.setInterval(() => {
        if (opts?.scope && !opts.scope.current()) return;
        const elapsed = panel.querySelector(".booking-elapsed strong");
        if (elapsed) elapsed.textContent = elapsedLabel(currentRunEvents(st?.recent_events ?? [], ev), true);
        if (++tick % 5 === 0) void pollWhileRunning(panel, opts?.scope);
      }, 1000),
    );
  }

  const stopButton = panel.querySelector<HTMLButtonElement>("#disarm_hang");
  if (stopButton) stopButton.onclick = async () => {
    stopButton.disabled = true;
    try {
      const out = await invoke<string>("disarm_hang");
      toastSuccess(running ? "正在停止" : "已取消定时预约", out || undefined);
      if (!opts?.scope || opts.scope.current()) void renderLive(panel, { statusOnly: true });
    } catch (e) {
      log.warn("live", "disarm_hang_failed", { err: String(e) });
      toastError("取消失败", String(e));
      stopButton.disabled = false;
    }
  };

}

let polling = false;
async function pollWhileRunning(panel: Element, scope?: PageScope) {
  if (tab !== "live" || (scope && !scope.current()) || polling) {
    return;
  }
  polling = true;
  try {
    const s = await invoke<LiveStatus>("get_live_status");
    if (tab !== "live" || (scope && !scope.current())) return;
    const wasRunning = Boolean(liveStatusCache?.running);
    ingestLiveStatus(s);
    paintLiveBody(panel, s, liveWxCache, {
      knownStatus: true,
      knownWx: liveWxCache !== null,
      scope,
    });
    if (!s.running && wasRunning) {
      log.info("live", "vision_idle", {
        action: s.last_event?.action ?? "",
        result: s.last_event?.result ?? "",
      });
    }
  } catch (e) {
    log.warn("live", "poll_status_failed", { err: String(e) });
  } finally { polling = false; }
}

export async function renderLive(panel: Element, opts?: RenderLiveOpts) {
  const scope = mountPage(panel);
  scope.onDispose(clearLiveTick);
  const gen = bumpLiveRenderGen();
  const statusOnly = Boolean(opts?.statusOnly);

  const endPaint = log.time("live", "paint_cache");
  if (opts?.status) ingestLiveStatus(opts.status);
  paintLiveBody(panel, opts?.status ?? liveStatusCache, liveWxCache, {
    knownStatus: liveStatusCache !== null,
    knownWx: liveWxCache !== null,
    scope,
  });
  endPaint();
  scope.onDispose(bookingPlans.subscribe(snapshot => {
    if (scope.current()) paintPlanContext(panel, liveStatusCache, snapshot);
  }));
  void bookingPlans.read().catch(error => log.warn("plans", "read_failed", { err: String(error) }));

  let st: LiveStatus | null = liveStatusCache;
  let wx: WechatProbe | null = liveWxCache;
  let gotStatus = liveStatusCache !== null;
  let gotWx = liveWxCache !== null;

  const stillMine = () => scope.current() && gen === liveRenderGen && tab === "live";

  const patch = () => {
    if (!stillMine()) return;
    paintLiveBody(panel, st, wx, { knownStatus: gotStatus, knownWx: gotWx, scope });
  };

  if (opts?.status && statusOnly) return;
  const endStatus = log.time("live", "get_live_status");
  void invoke<LiveStatus>("get_live_status")
    .then((s) => {
      endStatus();
      if (!stillMine()) return;
      st = s;
      gotStatus = true;
      ingestLiveStatus(s);
      patch();
    })
    .catch((e) => {
      endStatus();
      if (!stillMine()) return;
      log.error("live", "get_live_status_failed", { err: String(e) });
      if (!gotStatus) {
        panel.innerHTML = `<div class="page-stack"><p class="msg err">${String(e)}</p></div>`;
      }
    });

  if (statusOnly) return;

  const endWx = log.time("live", "probe_wechat");
  void invoke<WechatProbe>("probe_wechat")
    .then((w) => {
      endWx();
      if (!stillMine()) return;
      wx = w;
      gotWx = true;
      setLiveWxCache(w);
      patch();
    })
    .catch(() => {
      endWx();
      if (!stillMine()) return;
      log.warn("live", "probe_wechat_failed");
      wx = {
        running: false,
        title: null,
        miniprogram: false,
        message: "无法检测微信状态",
      };
      gotWx = true;
      setLiveWxCache(wx);
      patch();
    });
}
