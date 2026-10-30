import { renderPlanQueue } from "../ui/plan-queue";
import { bookingPlans, type PlansSnapshot } from "../lib/booking-plans-store";
import { invoke } from "@tauri-apps/api/core";
import type { BookingTask, LiveStatus, RunEvent, WechatProbe } from "../types";
import { ingestLiveStatus } from "../lib/run-feedback";
import { outcomeFromEvent } from "../lib/run-outcome";
import { clearLiveTick, setRunIndicator } from "../lib/run-indicator";
import {
  formatFallbackLabel,
  formatFireAtHuman,
  formatTargetBrief,
  formatTaskSportVenue,
  joinParts,
} from "../lib/format-human";
import { formatRemaining } from "../lib/time";
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
    return `<li class="${index === rows.length - 1 && running ? "active" : ""}"><time>${escText(time)}</time><span>${escText(progressMessage(event))}</span>${duration !== null ? `<small>${duration.toFixed(1)}s</small>` : ""}</li>`;
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
  const state = running ? "running" : uncertain ? "unconfirmed" : failed ? "failed" : result?.verified ? "success" : "finished";
  const confirming = running && latestPhase && progressStep(latestPhase) === "order";
  const title = running ? recheckOnly ? "正在再次查单" : confirming ? "正在确认预约结果" : st?.operation === "scheduled" && st.armed ? "定时预约准备中" : authOnly ? "正在检查体育登录" : prepareOnly ? "正在检查预约条件" : "预约进行中" : outcome?.title ?? "预约流程已结束";
  const detail = running ? (last ? progressMessage(last) : "正在启动…") : outcome?.detail ?? (last ? progressMessage(last) : "");
  const court = result?.court ?? selected ?? target?.court;
  return `<section class="booking-execution ${state}${running ? "" : " booking-recent"}" aria-label="预约执行状态">
    <div class="booking-execution-head"><span class="booking-state-icon" aria-hidden="true">${running ? '<span class="booking-spinner"></span>' : state === "success" ? "✓" : failed ? "!" : "·"}</span><div class="booking-head-copy"><span class="booking-eyebrow">${recheckOnly ? running ? "只读查单" : "最近一次查单" : running ? "正在执行" : "最近一次预约"}</span><h2 role="status">${escText(title)}</h2></div><span class="booking-elapsed">${running ? "已用时" : "总用时"}<strong>${elapsedLabel(events, running)}</strong></span>${running ? '<button type="button" class="btn ghost booking-stop" id="disarm_hang">停止</button>' : ""}</div>
    <p class="booking-current-detail">${escText(detail)}</p>${confirming ? '<p class="booking-current-detail">正在只读查单；停止只会结束后续检查，已经发送的预约不会被撤回。</p>' : ""}
    <div class="booking-target"><strong>${escText(venueLabel)}</strong><div><span>${escText(date)}</span><span>${escText(slot)}</span><span>${escText(court ? `${court} 号场地` : "自动选择场地")}</span></div>${result?.order_id ? `<small>订单号 ${escText(String(result.order_id))}</small>` : ""}</div>
    <ol class="booking-track${recheckOnly ? " recheck-only" : authOnly ? " auth-only" : ""}" aria-label="执行步骤">${steps}</ol>
    <details class="booking-details" ${detailsOpen ? "open" : ""}><summary><span>过程记录</span><span>${rows.length} 条 <span class="booking-chevron">⌄</span></span></summary><ol class="live-timeline" aria-label="预约过程记录">${timeline}</ol><p class="booking-run-id">${escText(`执行编号 ${last?.run_id ?? ""}`)}</p></details>
  </section>`;
}

function recoveryHtml(event: RunEvent | null): string {
  const code = event?.result ?? "";
  if (code.startsWith("RUNTIME_") && code !== "RUNTIME_CLOSE_FAILED") return '<div class="task-busy-note"><span>请重启电脑微信后重试；仍失败请导出诊断日志。</span><button type="button" class="btn ghost" data-tab="settings">前往设置</button></div>';
  const login = ["LOGIN_CONFIRM_REQUIRED", "SESSION_EXPIRED", "NO_ENTRY", "RUNTIME_UNAVAILABLE", "RUNTIME_UNSUPPORTED", "LOGIN_FOCUS_REQUIRED"].includes(code);
  const uncertain = ["SUBMIT_UNKNOWN", "RESULT_UNCONFIRMED", "http_submitted", "ORDER_RECHECK_UNCONFIRMED"].includes(code);
  if (login || uncertain) return `<div class="task-busy-note"><span>${uncertain ? "可在下方待确认记录点击「再次查单」，或在小程序「我的预约」核对；不会自动重发。" : "完成体育登录并进入场馆预约后，可以重新执行原任务。"}</span><button type="button" class="btn ghost" id="open_booking">打开小程序${uncertain ? "确认" : "登录"}</button></div>`;
  if (event?.action === "fail") return '<div class="task-busy-note"><span>可调整日期、场馆或备选策略，再尝试预约。</span><button type="button" class="btn ghost" data-tab="setup">修改预约目标</button></div>';
  return "";
}

function taskBriefHtml(t: BookingTask | null, opts?: { withSetupCta?: boolean }): string {
  if (!t) {
    const cta = opts?.withSetupCta
      ? `<p class="live-brief-cta"><button type="button" class="btn primary" data-tab="setup">去配置任务</button></p>`
      : "";
    return `<div class="live-brief empty-brief activity-empty"><span class="activity-empty-icon" aria-hidden="true"><svg viewBox="0 0 48 48" fill="none"><circle cx="24" cy="24" r="16"/><path d="M24 14v11l7 4"/></svg></span>
      <p class="live-brief-title">尚未配置任务</p>
      <p class="live-brief-sub">添加预约后，在这里查看执行进度</p>
      ${cta}
    </div>`;
  }
  return `<div class="live-brief">
    <p class="live-brief-title">${escText(formatTaskSportVenue(t))}</p>
    <p class="live-brief-sub">${escText(
      joinParts(formatTargetBrief(t), formatFallbackLabel(t)),
    )}</p>
  </div>`;
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
  if (!host || (!snapshot.view && !snapshot.error)) return;
  let hasPlans: boolean;
  if (snapshot.error) {
    host.textContent = `暂未读取到长期计划：${String(snapshot.error)}`;
    hasPlans = true;
  } else hasPlans = renderPlanQueue(host, snapshot.view!);
  const task = status?.task;
  const context = panel.querySelector<HTMLElement>("#live_single_context");
  if (context && !status?.armed && !status?.running) context.hidden = task ? Boolean(task.plan_id) : hasPlans;
}

function paintLiveBody(
  panel: Element,
  st: LiveStatus | null,
  wx: WechatProbe | null,
  opts?: { knownStatus?: boolean; knownWx?: boolean; scope?: PageScope },
) {
  const previousTimeline = panel.querySelector<HTMLElement>(".live-timeline");
  const previousQueue = panel.querySelector("#live_plan_queue")?.innerHTML ?? "";
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
  const fireAt = st?.fire_at || t?.fire_at || null;
  const wxRunning = Boolean(wx?.running);
  const busy = armed || running;

  if (knownStatus) {
    setRunIndicator(busy);
  }

  const wxText = !knownWx ? "检测中…" : wxRunning ? "已运行" : "未运行";
  const executionBlock = executionHtml(st, detailsOpen);
  const outcomeBlock = !running && !executionBlock ? outcomeBarHtml(ev) : "";

  let mainBlock = "";

  if (armed && fireAt) {
    mainBlock = `
      <div class="settings-group">
        <p class="settings-group-title">挂机等待</p>
        <div class="settings-group-body">
          <div class="live-hero">
            <p class="live-countdown-label">距离开抢</p>
            <p class="live-countdown-value" id="live_remain">计算中…</p>
            <p class="live-countdown-sub">${escText(formatFireAtHuman(fireAt))}${t?.executor === "http" ? "　提前 1 分钟准备登录与 H5" : ""}</p>
          </div>
          ${taskBriefHtml(t)}
          ${!running ? `<div class="settings-cell">
            <div>
              <p class="settings-cell-title">取消挂机</p>
              <p class="settings-cell-desc">停止等待，不会到点开抢</p>
            </div>
            <button type="button" class="btn ghost" id="disarm_hang">取消挂机</button>
          </div>` : ""}
        </div>
      </div>`;
  } else if (running) {
    mainBlock = "";
  } else {
    mainBlock = `
      <div class="settings-group">
        ${t ? '<p class="settings-group-title">单次预约草稿</p>' : ""}
        <div class="settings-group-body well-flush">
          ${taskBriefHtml(t, { withSetupCta: true })}
        </div>
      </div>`;
  }

  patchHtml(panel, `
    <div class="page-stack live-page" data-idle="${!busy && !t && !executionBlock && !outcomeBlock}">
      <div class="page-heading"><div><h2>运行进度</h2><p>当前进度、执行队列与预约结果</p></div><div class="live-heading-actions">${envRowHtml(wxText, wxRunning)}<button type="button" class="btn ghost" id="live_configuration" ${t ? "" : "hidden"} data-tab="${t?.plan_id ? "plans" : "setup"}" ${t?.plan_id ? `data-plan-id="${escText(t.plan_id)}"` : ""}>${t?.plan_id ? "查看本项计划" : busy ? "查看配置" : "修改单次预约"}</button></div></div>
      <section id="live_plan_queue" class="live-plan-queue">${previousQueue}</section>
      ${outcomeBlock}
      ${executionBlock}
      ${!running ? recoveryHtml(ev) : ""}
      <div id="pending_order_checks"></div>
      <div id="live_single_context" ${!busy && (!t || t.plan_id) ? "hidden" : ""}>${mainBlock}</div>
    </div>
    ${stopping ? '<p class="task-busy-note" role="status">正在停止，请等待已发送请求的结果确认。</p>' : ""}
  `);
  paintPlanContext(panel, st, bookingPlans.snapshot());
  const pendingHost=panel.querySelector<HTMLElement>("#pending_order_checks");
  if (pendingHost) void mountOrderRecheck(pendingHost,busy,()=>!opts?.scope || opts.scope.current());
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

  if (armed && fireAt) {
    const remainEl = panel.querySelector("#live_remain");
    const tick = () => {
      const ms = Date.parse(fireAt) - Date.now();
      if (remainEl) remainEl.textContent = formatRemaining(ms);
    };
    tick();
    setLiveTickTimer(window.setInterval(tick, 1000));
  } else if (running) {
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
