import { invoke } from "@tauri-apps/api/core";
import { liveStatusCache, subscribeLiveStatus } from "../state";
import type { LiveStatus } from "../types";
import { localDateISO, addDaysISO } from "../lib/time";
import { askConfirm } from "./confirm-ask";
import { resetAvailabilitySessionCache } from "../lib/availability-store";
import { availabilityAuthNeeded } from "../lib/availability";
import { toastError, toastSuccess } from "./toast";
import { currentRunEvents } from "../lib/booking-progress";

type Order = {order_id:string; details_id:string; venue:string; sport:string; court:string; date:string; start:string; end:string; can_cancel:boolean};
type OrdersResult = {ok:boolean; account_fingerprint?:string; orders?:Order[]; message?:string; code?:string; phase?:string; state?:string; cached?:boolean; fetched_at?:string};
const RETRY_MAX = 900000;
export function orderSnapshotFresh(fetched:number, now=Date.now()):boolean {
  const beijing=new Date(now+8*3600000);
  const minute=beijing.getUTCHours()*60+beijing.getUTCMinutes();
  const ttl=minute>=420 && minute<435 ? 300000 : 1800000;
  return fetched>0 && now>=fetched && now-fetched<ttl;
}
export function orderChangeTime(value:LiveStatus):number {
  const events=currentRunEvents(value.recent_events ?? [],value.last_event);
  if(value.last_event) events.push(value.last_event);
  return events.reduce((latest,event)=> {
    const phase=event.http?.phase ?? event.result ?? "";
    const changed=["submitting","order_accepted","verifying_order"].includes(phase)
      || ["http_verified","http_already_reserved","http_order_rechecked","http_submitted","SUBMIT_UNKNOWN","RESULT_UNCONFIRMED","ORDER_RECHECK_VERIFIED"].includes(event.result ?? "");
    const time=Date.parse(event.ts);
    return changed && Number.isFinite(time) ? Math.max(latest,time) : latest;
  },0);
}
const escape = (value:string) => value.replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
const refreshIcon = '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M13 6A5.2 5.2 0 1 0 13 10M13 2.5V6H9.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';

export function orderDay(date:string, now=new Date(Date.now())) {
  const today=localDateISO(now);
  const relative=["今天","明天","后天"];
  for(let offset=0;offset<relative.length;offset++) if(date===addDaysISO(today,offset)) return relative[offset];
  return date.startsWith(today.slice(0,4)) ? date.slice(5).replace("-","/") : date.replaceAll("-","/");
}
export function orderPlace(row:Pick<Order,"venue"|"sport"|"court">) {
  const venue=row.venue.trim(), sport=row.sport.trim(), court=row.court.trim();
  let number=court.replace(/\s/g,"");
  if(sport && number.startsWith(sport)) number=number.slice(sport.length);
  const numbered=/^(?:第)?(?:场地|球场|球台|台|桌)?[A-Za-z]?\d+(?:号)?(?:场地|球场|球台|台|桌|场)?$/.test(number);
  if(numbered) return {name:venue || sport || "场馆预约",court:/^\d+$/.test(court) ? `${court} 号场地` : court};
  return {name:court && court!==sport ? court : venue || sport || "场馆预约",court:""};
}
export function orderEnded(row:Pick<Order,"date"|"end">,now=Date.now()) {
  const end=Date.parse(`${row.date}T${row.end}:00+08:00`);
  return !Number.isFinite(end) || end<=now;
}
export function orderCanCancel(row:Pick<Order,"date"|"start"|"can_cancel">,now=Date.now()) {
  return row.can_cancel && Date.parse(`${row.date}T${row.start}:00+08:00`)-now>3600000;
}
const cancelIcon='<svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2"/><path d="m6 6 4 4m0-4-4 4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>';

export function mountMyOrders(host:HTMLElement, options: { popover?: boolean; icon?: string } = {}) {
  host.classList.toggle("orders-popover", !!options.popover);
  let rows:Order[] = [], account = "", lastSuccess = 0, lastAttempt = -Infinity;
  let loading = false, recovering = false, confirming = false, cancelling = false, loginRequired = false, known = false, busy = true, message = "";
  let queued = false;
  let expanded = false;
  let closeTimer: ReturnType<typeof setTimeout> | undefined;
  const clearCloseTimer = () => { clearTimeout(closeTimer); closeTimer = undefined; };
  const setExpanded = (value: boolean) => {
    clearCloseTimer();
    expanded = value;
    host.classList.toggle("is-expanded", value);
    host.querySelector<HTMLElement>(".my-orders-popup")?.toggleAttribute("hidden", !value);
    host.querySelector(".my-orders-toggle")?.setAttribute("aria-expanded", String(value));
  };
  let changedAt = 0, dirty = false, failures = 0, retryAt = 0;
  const label = (row:Order) => `${row.venue}${row.court ? `、${row.court}` : ""}，${row.date} ${row.start}–${row.end}`;
  let paintedPolicy="";
  const policyKey=()=>`${localDateISO(new Date(Date.now()))}:${rows.map(row=>`${row.order_id}/${row.details_id}:${orderEnded(row)}:${orderCanCancel(row)}`).join(";")}`;
  function paint() {
    rows=rows.filter(row=>!orderEnded(row));
    paintedPolicy=policyKey();
    const disabled = busy || !known || loading || recovering || confirming || cancelling;
    const visibleStatus = !loading && !recovering
      ? loginRequired ? message || "登录状态已失效，点击右侧按钮重新登录" : busy && known ? "预约进行中，暂停更新" : message
      : "";
    const refreshLabel = recovering ? "正在重新登录" : loading ? "正在更新预约" : loginRequired ? "重新登录并刷新预约" : "刷新我的预约";
    const liveStatus = recovering ? "正在恢复微信登录" : loading ? "正在更新预约" : visibleStatus || "预约列表已更新";
    const collapsedLabel = loginRequired ? "需登录" : loading || recovering ? "更新中" : failures ? "查看状态" : "展开";
    host.classList.toggle("is-expanded", expanded);
    host.innerHTML = `${options.popover ? `<button type="button" class="my-orders-toggle header-orders-button" aria-expanded="${expanded}" aria-controls="my_orders_popup">${options.icon ?? ""}<span>我的预约</span><span class="orders-count">${loginRequired ? "需登录" : loading || recovering ? "…" : failures ? "!" : rows.length}</span></button><div class="my-orders-popup" id="my_orders_popup" ${expanded ? "" : "hidden"}>` : ""}<div class="my-orders-head"><h2>我的预约${rows.length ? `<span>${rows.length}</span>` : ""}</h2>${!options.popover ? `<button type="button" class="my-orders-toggle" aria-expanded="${expanded}" aria-controls="my_orders_body">我的预约${rows.length ? ` ${rows.length}` : ""}<span>${expanded ? "收起" : collapsedLabel}</span></button>` : ""}<button type="button" class="order-icon ${loading || recovering ? "is-loading" : ""}" data-orders-refresh aria-label="${refreshLabel}" aria-busy="${loading || recovering}" ${disabled ? "disabled" : ""}>${refreshIcon}</button></div>
      <div class="my-orders-body" id="my_orders_body">${visibleStatus ? `<p class="my-orders-status" role="status">${escape(visibleStatus)}</p>` : `<span class="sr-only" role="status" aria-live="polite">${escape(liveStatus)}</span>`}
      <div class="my-orders-list">${rows.map((row,index) => {const place=orderPlace(row);const cancellable=orderCanCancel(row);const cancelReason=!cancellable ? (Date.parse(`${row.date}T${row.start}:00+08:00`)-Date.now()<=3600000 ? "开始前1小时内不可取消" : "此预约暂不可取消") : "";return `<article class="my-order"><div class="my-order-head"><span class="my-order-name">${escape(place.name)}</span><button type="button" class="order-cancel" data-order-cancel="${index}" aria-label="${escape(`取消预约：${label(row)}`)}" ${disabled || loginRequired || !cancellable ? "disabled" : ""}>${options.popover ? "取消预约" : cancelIcon}</button></div><p class="my-order-time"><span class="my-order-day">${escape(orderDay(row.date))}</span><strong>${escape(row.start)}–${escape(row.end)}</strong></p>${place.court ? `<p class="my-order-court">${escape(place.court)}</p>` : ""}${cancelReason ? `<p class="my-order-cancel-reason">${cancelReason}</p>` : ""}</article>`;}).join("")}</div></div>${options.popover ? "</div>" : ""}`;
    host.querySelector<HTMLButtonElement>(".my-orders-toggle")!.onclick = event => {
      if (options.popover) { setExpanded(true); return; }
      expanded = !expanded; host.classList.toggle("is-expanded", expanded);
      const button = event.currentTarget as HTMLButtonElement;
      button.setAttribute("aria-expanded", String(expanded));
      button.querySelector("span")!.textContent = expanded ? "收起" : collapsedLabel;
    };
    host.querySelector<HTMLButtonElement>("[data-orders-refresh]")!.onclick = () => void refresh(true);
    host.querySelectorAll<HTMLButtonElement>("[data-order-cancel]").forEach(button => {
      button.onclick = () => void cancelRow(rows[Number(button.dataset.orderCancel)]);
    });
  }
  if (options.popover) {
    const close = () => setExpanded(false);
    host.addEventListener("pointerenter", event => { if (event.pointerType === "mouse") setExpanded(true); });
    host.addEventListener("pointerleave", event => {
      if (event.pointerType !== "mouse") return;
      clearCloseTimer();
      // Allow the pointer to cross the space between the header and its card.
      closeTimer = setTimeout(() => {
        if (!confirming && !cancelling && !host.querySelector(".my-orders-popup")?.contains(document.activeElement)) close();
      }, 220);
    });
    host.addEventListener("focusout", event => {
      if (event.relatedTarget && !host.contains(event.relatedTarget as Node) && !host.matches(":hover") && !confirming && !cancelling) close();
    });
    document.addEventListener("click", event => { if (expanded && !event.composedPath().includes(host)) close(); });
    host.addEventListener("keydown", event => {
      if (event.key === "Escape" && expanded) {
        event.preventDefault(); event.stopPropagation(); close();
        host.querySelector<HTMLButtonElement>(".my-orders-toggle")?.focus();
      }
    });
  }
  async function refresh(manual = false, force = false) {
    if (!host.isConnected || document.hidden || !known || busy || recovering || confirming || cancelling) { if(force) queued = true; return; }
    if (loginRequired) { if(manual) await recoverLogin(); return; }
    if (loading) { if(force) queued = true; return; }
    if (!manual && queued) { force=true;queued=false; }
    if (!manual && !force && (orderSnapshotFresh(lastSuccess) || Date.now()<retryAt)) return;
    if (!force && Date.now()-lastAttempt < (manual ? 10000 : 60000)) {
      return;
    }
    let recoverAfterRefresh=false;
    loading = true; lastAttempt = Date.now(); message = ""; paint();
    try {
      const result = await invoke<OrdersResult>("get_my_orders",{force:manual || force});
      if (!result.ok) {
        if (["ACCOUNT_CHANGED","ACCOUNT_MISMATCH"].includes(result.code ?? "")) { rows=[];account=""; }
        if (availabilityAuthNeeded(result)) {loginRequired=true;recoverAfterRefresh=manual;}
        throw new Error(result.message ?? "请先在预约页重新登录，再刷新订单");
      }
      rows = [...(result.orders ?? [])].sort((a,b) => `${a.date} ${a.start}`.localeCompare(`${b.date} ${b.start}`));
      const fetched=Date.parse(result.fetched_at ?? "");
      account = result.account_fingerprint ?? "";
      lastSuccess = Number.isFinite(fetched) && fetched<=Date.now() ? fetched : Date.now();
      failures=0;retryAt=0;loginRequired = false;
      if (!manual && !force && result.cached && changedAt>lastSuccess) queued=true;
      message = rows.length ? "" : "暂无待使用的预约";
    } catch(error) {
      retryAt=Date.now()+Math.min(RETRY_MAX,60000*2**Math.min(failures++,4));
      if(loginRequired) message = "";
      else {
        message = rows.length ? "更新失败，显示上次结果" : "暂未加载，请刷新重试";
        if(manual) toastError("预约未能刷新",String(error));
      }
    } finally {
      loading = false; paint();
      if(recoverAfterRefresh) {queued=false;await recoverLogin();}
      else if(queued) {queued=false;void refresh(false,true);}
    }
  }
  async function recoverLogin() {
    if (!host.isConnected || document.hidden || !known || busy || loading || recovering || confirming || cancelling) return;
    recovering = true; message = ""; paint();
    try {
      const result = await invoke<OrdersResult>("reauthenticate_availability",{operationId:crypto.randomUUID(),expectedAccount:account || null});
      if (!result.ok) {
        if (result.code === "ACCOUNT_CHANGED") { rows=[]; account=""; }
        loginRequired = availabilityAuthNeeded(result) || result.code === "LOGIN_CONFIRM_REQUIRED" || result.code === "ACCOUNT_CHANGED";
        message = result.message ?? "小程序导航或登录核验未完成，请查看当前页面后重试";
        return;
      }
      loginRequired = false;
      resetAvailabilitySessionCache();
      window.dispatchEvent(new CustomEvent("orders-authenticated",{detail:{source:"orders"}}));
    } catch(error) {
      message = "微信登录恢复失败，点击右侧按钮重试";
      toastError("登录未完成",String(error));
    } finally {
      recovering = false;
      paint();
      if(queued && !busy && !document.hidden) {queued=false;void refresh(false,true);}
    }
  }
  async function cancelRow(row:Order) {
    if (!row || busy || loading || recovering || loginRequired || confirming || cancelling || !orderCanCancel(row) || orderEnded(row) || !account) return;
    confirming=true;paint();
    const originalAccount=account;
    let confirmed=false;
    try {confirmed = await askConfirm({title:"取消这场预约？",lead:label(row),confirmLabel:"确认取消预约",cancelLabel:"保留预约",danger:true});}
    finally {confirming=false;}
    if(!confirmed || busy || originalAccount!==account) {paint();return;}
    if(!orderCanCancel(row) || orderEnded(row)) {paint();toastError("无法取消预约","已进入开始前1小时，不能取消预约");return;}
    cancelling=true;message="正在取消，请稍候…";paint();
    let confirmedCancellation=false;
    try {
      const result=await invoke<OrdersResult>("cancel_my_order",{accountFingerprint:originalAccount,orderId:row.order_id,detailsId:row.details_id});
      if(!result.ok || result.state!=="cancelled") throw new Error(result.message ?? "取消结果未确认，请刷新或到小程序核对，勿重复取消");
      rows=rows.filter(item=>item!==row);
      confirmedCancellation=true;
      message=rows.length ? "" : "暂无待使用的预约";
      toastSuccess("预约已取消");
    } catch(error) {toastError("取消未能确认",String(error));}
    finally {
      cancelling=false;paint();
      if(!confirmedCancellation || queued) {queued=false;await refresh(false,true);}
    }
  }
  function status(value:LiveStatus) {
    const wasBusy=busy,wasKnown=known;const nextBusy=value.running || value.armed || Boolean(value.stopping);
    const change=orderChangeTime(value);
    if(change>changedAt) {changedAt=change;if(wasKnown) dirty=true;}
    known=true;busy=nextBusy;
    if(!wasKnown || wasBusy!==busy) paint();
    if(!busy && (!wasKnown || wasBusy || queued || dirty)) {
      const force=queued || dirty;queued=false;dirty=false;void refresh(false,force);
    }
  }
  subscribeLiveStatus(status);
  if(liveStatusCache) status(liveStatusCache);
  else void invoke<LiveStatus>("get_live_status").then(status).catch(()=>{message="暂未加载，请稍后重试";known=true;busy=false;paint();});
  window.addEventListener("focus",()=>void refresh());
  document.addEventListener("visibilitychange",()=>{if(!document.hidden) void refresh();});
  window.addEventListener("orders-authenticated",()=>{loginRequired=false;void refresh(false,true);});
  // Local wake-up only: freshness, visibility and booking gates decide whether to query.
  window.setInterval(()=>void refresh(),60000);
  window.setInterval(()=>{if(!loading && !recovering && !confirming && !cancelling && policyKey()!==paintedPolicy) {if(rows.some(row=>orderEnded(row)) && !rows.some(row=>!orderEnded(row))) message="暂无待使用的预约";paint();}},1000);
  paint();
}
