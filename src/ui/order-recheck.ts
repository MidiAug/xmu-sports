import { invoke } from "@tauri-apps/api/core";
import { toastError, toastInfo } from "./toast";

type PendingOrder = {
  request_ref: string; can_recheck: boolean; reason?: string; last_checked_at?: string;
  target?: {sport?: string; venue?: string; date?: string; time?: string; end?: string; court?: string};
};
const escape = (value: string) => value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
const generations = new WeakMap<HTMLElement, number>();

export async function mountOrderRecheck(host: HTMLElement, busy: boolean, current: () => boolean) {
  const generation = (generations.get(host) ?? 0) + 1;
  generations.set(host, generation);
  const owns = () => host.isConnected && current() && generations.get(host) === generation;
  try {
    const rows = await invoke<PendingOrder[]>("list_pending_orders");
    if (!owns()) return;
    if (!rows.length) {host.innerHTML = ""; return;}
    host.innerHTML = `<section class="settings-group" aria-label="待确认预约"><p class="settings-group-title">待确认预约</p><p class="settings-cell-desc">按原提交账号与实际场次只读核对，与当前预约配置无关；不会再次下单。</p><div class="settings-group-body">${rows.map((row, index) => {
      const goal = row.target;
      const label = goal ? [goal.venue, goal.sport, goal.date, `${goal.time ?? ""}–${goal.end ?? ""}`, goal.court ? `${goal.court} 号场地` : ""].filter(Boolean).join("　") : "原提交记录";
      return `<div class="settings-cell"><div><p class="settings-cell-title">${escape(label)}</p><p class="settings-cell-desc">${escape(row.can_recheck ? "只查询原记录，不提交、不切换候选" : row.reason ?? "请在小程序核对")}</p></div><button type="button" class="btn ghost" data-order-recheck="${index}" ${busy || !row.can_recheck ? "disabled" : ""}>再次查单</button></div>`;
    }).join("")}</div><p role="status" class="settings-cell-desc" data-recheck-status></p></section>`;
    host.querySelectorAll<HTMLButtonElement>("[data-order-recheck]").forEach(button => {
      button.onclick = async () => {
        const row = rows[Number(button.dataset.orderRecheck)];
        if (!row?.can_recheck || busy || !owns()) return;
        // Disable every local button; the host also serializes this with all
        // booking/scheduler operations, including clicks after a page repaint.
        host.querySelectorAll<HTMLButtonElement>("[data-order-recheck]").forEach(item => {item.disabled = true;});
        button.textContent = "正在查单…";
        const status = host.querySelector<HTMLElement>("[data-recheck-status]");
        if (status) status.textContent = "正在校验原账号并读取预约列表和详情，不提交预约。";
        try {
          await invoke("recheck_order", {requestRef: row.request_ref});
          // The result is persisted as a separate read-only run, displayed by
          // normal run-changed feedback. Do not toast an unverified success.
        } catch (error) {
          if (owns()) toastError("再次查单未完成", String(error));
          if (status?.isConnected) status.textContent = "原预约仍待确认，未重复下单。";
        } finally {
          if (owns()) await mountOrderRecheck(host, false, current);
        }
      };
    });
  } catch (error) {
    if (owns()) {
      host.innerHTML = '<p class="task-busy-note">待确认记录读取失败；请在小程序核对，程序不会因此重新下单。</p>';
      toastInfo("待确认记录读取失败", String(error));
    }
  }
}
