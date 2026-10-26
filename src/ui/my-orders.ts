import { invoke } from "@tauri-apps/api/core";
import { liveStatusCache, subscribeLiveStatus } from "../state";
import type { LiveStatus } from "../types";
import { localDateISO, addDaysISO } from "../lib/time";
import { askConfirm } from "./confirm-ask";
import { toastError, toastSuccess } from "./toast";

type Order = {order_id:string; details_id:string; venue:string; sport:string; court:string; date:string; start:string; end:string; can_cancel:boolean};
type OrdersResult = {ok:boolean; account_fingerprint?:string; orders?:Order[]; message?:string; code?:string; state?:string};
const escape = (value:string) => value.replaceAll("&","&amp;").replaceAll("<","&lt;").replaceAll(">","&gt;").replaceAll('"',"&quot;");
const refreshIcon = '<svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M13 6A5.2 5.2 0 1 0 13 10M13 2.5V6H9.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>';

export function orderDay(date:string, now=new Date(Date.now())) {
  const today=localDateISO(now);
  const relative=["今天","明天","后天"];
  for(let offset=0;offset<relative.length;offset++) if(date===addDaysISO(today,offset)) return relative[offset];
  return date.startsWith(today.slice(0,4)) ? date.slice(5).replace("-","/") : date.replaceAll("-","/");
}
const cancelIcon='<svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true"><circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.2"/><path d="m6 6 4 4m0-4-4 4" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>';

export function mountMyOrders(host:HTMLElement) {
  let rows:Order[] = [], account = "", lastSuccess = 0, lastAttempt = -Infinity;
  let loading = false, confirming = false, cancelling = false, known = false, busy = true, message = "正在读取预约…";
  let queued = false;
  const label = (row:Order) => `${row.venue}${row.court ? `、${row.court}` : ""}，${row.date} ${row.start}–${row.end}`;
  function paint() {
    const disabled = busy || !known || loading || confirming || cancelling;
    host.innerHTML = `<div class="my-orders-head"><h2>我的预约${rows.length ? `<span>${rows.length}</span>` : ""}</h2><button type="button" class="order-icon ${loading ? "is-loading" : ""}" data-orders-refresh title="刷新预约，后台每 5 分钟更新" aria-label="刷新我的预约" ${disabled ? "disabled" : ""}>${refreshIcon}</button></div>
      ${(busy && known) || message ? `<p class="my-orders-status" role="status">${escape(busy && known ? "预约进行中，暂停更新" : message)}</p>` : '<span class="sr-only" role="status">预约已更新</span>'}
      <div class="my-orders-list">${rows.map((row,index) => `<article class="my-order" title="${escape(label(row))}"><div class="my-order-head"><span class="my-order-sport">${escape(row.sport || "场馆预约")}</span><button type="button" class="order-cancel" data-order-cancel="${index}" aria-label="${escape(`取消预约：${label(row)}`)}" title="${row.can_cancel ? "取消此预约" : "当前场次暂无法取消，请到小程序核对"}" ${disabled || !row.can_cancel ? "disabled" : ""}>${cancelIcon}</button></div><p class="my-order-time"><span class="my-order-day">${escape(orderDay(row.date))}</span><strong>${escape(row.start)}–${escape(row.end)}</strong></p><p class="my-order-venue">${escape(row.venue)}</p>${row.court ? `<p class="my-order-court">${escape(row.court)}</p>` : ""}</article>`).join("")}</div>`;
    host.querySelector<HTMLButtonElement>("[data-orders-refresh]")!.onclick = () => void refresh(true);
    host.querySelectorAll<HTMLButtonElement>("[data-order-cancel]").forEach(button => {
      button.onclick = () => void cancelRow(rows[Number(button.dataset.orderCancel)]);
    });
  }
  async function refresh(manual = false, force = false) {
    if (!host.isConnected || document.hidden || !known || busy || confirming || cancelling) { if(force) queued = true; return; }
    if (loading) { if(force) queued = true; return; }
    if (!force && Date.now()-lastAttempt < (manual ? 10000 : 60000)) {
      if(manual) {message = "刚刚已刷新，请稍后再试";paint();} return;
    }
    loading = true; lastAttempt = Date.now(); message = "正在更新…"; paint();
    try {
      const result = await invoke<OrdersResult>("get_my_orders");
      if (!result.ok) {
        if (["ACCOUNT_CHANGED","ACCOUNT_MISMATCH"].includes(result.code ?? "")) { rows=[];account=""; }
        throw new Error(result.message ?? "请先在预约页重新登录，再刷新订单");
      }
      rows = [...(result.orders ?? [])].sort((a,b) => `${a.date} ${a.start}`.localeCompare(`${b.date} ${b.start}`));
      account = result.account_fingerprint ?? ""; lastSuccess = Date.now();
      message = rows.length ? "" : "暂无未核销预约";
      host.removeAttribute("title");
    } catch(error) {
      message = rows.length ? "更新失败，显示上次结果" : "暂未加载，请刷新重试";
      host.title = String(error);
      if(manual) toastError("预约未能刷新",String(error));
    } finally {
      loading = false; paint();
      if(queued) {queued=false;void refresh(false,true);}
    }
  }
  async function cancelRow(row:Order) {
    if (!row || busy || loading || confirming || cancelling || !row.can_cancel || !account) return;
    confirming=true;paint();
    const originalAccount=account;
    const confirmed = await askConfirm({title:"取消这场预约？",lead:label(row),confirmLabel:"确认取消预约",cancelLabel:"保留预约",danger:true});
    confirming=false;
    if(!confirmed || busy || originalAccount!==account) {paint();return;}
    cancelling=true;message="正在取消，请稍候…";paint();
    try {
      const result=await invoke<OrdersResult>("cancel_my_order",{accountFingerprint:originalAccount,orderId:row.order_id,detailsId:row.details_id});
      if(!result.ok || result.state!=="cancelled") throw new Error(result.message ?? "取消结果未确认，请刷新或到小程序核对，勿重复取消");
      rows=rows.filter(item=>item!==row);
      toastSuccess("预约已取消");
    } catch(error) {toastError("取消未能确认",String(error));}
    finally {cancelling=false;await refresh(false,true);}
  }
  function status(value:LiveStatus) {
    const wasBusy=busy;known=true;busy=value.running || value.armed;
    paint();
    if(!busy && (wasBusy || queued)) {queued=false;void refresh(false,true);}
  }
  subscribeLiveStatus(status);
  if(liveStatusCache) status(liveStatusCache);
  else void invoke<LiveStatus>("get_live_status").then(status).catch(()=>{message="暂未加载，请稍后重试";known=true;busy=false;paint();});
  window.addEventListener("focus",()=>{if(Date.now()-lastSuccess>=60000) void refresh();});
  document.addEventListener("visibilitychange",()=>{if(!document.hidden && Date.now()-lastSuccess>=60000) void refresh();});
  window.addEventListener("orders-authenticated",()=>void refresh(false,true));
  window.setInterval(()=>void refresh(),300000);
  let paintedDay=localDateISO(new Date(Date.now()));
  window.setInterval(()=>{const day=localDateISO(new Date(Date.now()));if(day!==paintedDay && !loading && !confirming && !cancelling) {paintedDay=day;paint();}},60000);
  paint();
}
