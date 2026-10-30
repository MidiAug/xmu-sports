import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ShellThemePref, Tab } from "../types";
import {
  app,
  maximized,
  runIndicatorOn,
  setMaximized,
  setTab,
  tab,
} from "../state";
import { applyTheme } from "../lib/theme";
import {
  startTargetDateCalibrationClock,
  whenBootCalibrationDone,
} from "../lib/calibrate-target-date";
import { log } from "../lib/log";
import { SVG, winButtonsHtml } from "./icons";
import { renderSetup } from "../pages/setup";
import { renderBookingPlans } from "../pages/booking-plans";
import { renderLive } from "../pages/live";
import { renderHistory } from "../pages/history";
import { renderSettings } from "../pages/settings";
import { disposePage } from "../lib/page-lifecycle";
import { flushTask, discardTaskDraft } from "../lib/task-store";
import { toastError } from "./toast";
import { askConfirm } from "./confirm-ask";

import { mountMyOrders } from "./my-orders";
import { mountSettingsUpdateIndicator } from "./version-update";
import { renderAbout } from "../pages/about";
import brandMark from "../assets/brand-mark.png";
import { calendarIcon } from "./sport-icons";

const win = getCurrentWindow();
let chromeBound = false;
let dateClockStarted = false;
let navigation = 0;

export async function syncMaximized() {
  try {
    const isMax = await win.isMaximized();
    setMaximized(isMax);
    app.querySelectorAll('[data-win="maximize"]').forEach((btn) => {
      btn.innerHTML = isMax ? SVG.restore : SVG.maximize;
      btn.setAttribute("aria-label", isMax ? "还原" : "最大化");
    });
  } catch {
    /* ignore */
  }
}

async function onWin(action: "minimize" | "maximize" | "close") {
  if (action === "minimize") await win.minimize();
  else if (action === "maximize") {
    await win.toggleMaximize();
    await syncMaximized();
  } else await win.close();
}

function bindChromeOnce() {
  if (chromeBound) return;
  chromeBound = true;

  app.addEventListener("click", (ev) => {
    const t = ev.target as HTMLElement | null;
    const tabBtn = t?.closest?.("[data-tab]") as HTMLButtonElement | null;
    if (tabBtn?.dataset.tab) {
      const next = tabBtn.dataset.tab as Tab;
      if (next === tab && !tabBtn.dataset.planId) return;
      void showTab(next, false, tabBtn.dataset.planId);
      return;
    }
    const themeBtn = t?.closest?.("[data-theme]") as HTMLButtonElement | null;
    if (themeBtn?.dataset.theme) {
      const next = themeBtn.dataset.theme as ShellThemePref;
      applyTheme(next);
      app.querySelectorAll("[data-theme]").forEach((b) => {
        const on = (b as HTMLButtonElement).dataset.theme === next;
        b.classList.toggle("on", on);
        b.setAttribute("aria-checked", String(on));
      });
      return;
    }
    const winBtn = t?.closest?.("[data-win]") as HTMLButtonElement | null;
    if (winBtn?.dataset.win) {
      void onWin(winBtn.dataset.win as "minimize" | "maximize" | "close");
    }
  });

  app.querySelector<HTMLElement>(".app-header")!.addEventListener("mousedown", (event) => {
    const target = event.target as Element;
    if (event.button !== 0 || target.closest("button,a,input,select,textarea,summary,[role=button],.my-orders-popup")) return;
    event.preventDefault();
    if (event.detail === 2) void onWin("maximize");
    else void win.startDragging().catch(error => log.warn("chrome", "window_drag_failed", { error: String(error) }));
  });

  void syncMaximized();
}

function updateNavActive() {
  app.querySelectorAll(".nav-tabs [data-tab]").forEach((btn) => {
    const id = (btn as HTMLButtonElement).dataset.tab;
    btn.classList.toggle("active", id === tab);
    if (id === tab) btn.setAttribute("aria-current", "page");
    else btn.removeAttribute("aria-current");
    if (id === "live") btn.classList.toggle("has-run-dot", runIndicatorOn);
  });
}

function updateWinButtons() {
  const wins = winButtonsHtml(maximized);
  app.querySelectorAll(".titlebar-right, .nav-win").forEach((el) => {
    el.innerHTML = wins;
  });
}

export function openAppUpdates() {void showTab("settings",true);}
async function showTab(next: Tab, about = false, planId?: string) {
  const resetScroll = next !== tab || about || !!planId;
  setTab(next);
  const request = ++navigation;
  log.debug("chrome", "show_tab", { tab: next });
  const end = log.time("chrome", "show_tab_done", { tab: next });
  updateNavActive();
  const panel = app.querySelector("#panel");
  if (!panel) {
    end();
    return;
  }
  try {
    disposePage(panel);
    if (resetScroll) panel.scrollTop = 0;
    await flushTask();
    if (request !== navigation || next !== tab) return;
    if (next === "setup") await renderSetup(panel);
    else if (next === "plans") await renderBookingPlans(panel, { focusPlanId: planId });
    else if (next === "live") await renderLive(panel);
    else if (next === "history") await renderHistory(panel);
    else if (about) await renderAbout(panel, () => { void showTab("settings"); });
    else await renderSettings(panel);
  } catch (error) {
    if (request !== navigation) return;
    toastError("页面未能加载", String(error));
    panel.innerHTML = '<div class="empty"><p class="empty-title">配置保存未完成</p><p class="empty-desc">修改仍被保留。可以重试保存，或放弃修改后重新载入。</p><button type="button" class="btn" id="retry_page">重试保存</button><button type="button" class="btn ghost" id="reload_page">重新载入配置</button></div>';
    panel.querySelector<HTMLButtonElement>("#retry_page")!.onclick = () => { void showTab(next, about); };
    panel.querySelector<HTMLButtonElement>("#reload_page")!.onclick = async () => {
      if (!await askConfirm({title:"重新载入配置？",lead:"尚未保存的修改将被放弃，恢复为最近保存的配置。",confirmLabel:"重新载入"})) return;
      discardTaskDraft(); void showTab(tab);
    };
  } finally {
    end();
  }
}

function mountShell() {
  const wins = winButtonsHtml(maximized);
  app.innerHTML = `
    <div class="shell">
      <div class="shell-row">
        <header class="nav app-header">
          <div class="nav-brand">
            <div class="nav-brand-text">
              <img class="brand-mark" src="${brandMark}" alt="" draggable="false" />
              <h1>XMU Sports</h1>
            </div>
          </div>
          <nav class="nav-tabs nav-primary" aria-label="主导航">
            <button type="button" data-tab="setup" aria-label="单次预约" class="${tab === "setup" ? "active" : ""}"><span class="nav-tab-label">预约</span></button>
            <button type="button" data-tab="plans" class="${tab === "plans" ? "active" : ""}"><span class="nav-tab-label">长期计划</span></button>
            <button type="button" data-tab="live" aria-label="运行" class="${tab === "live" ? "active" : ""}${runIndicatorOn ? " has-run-dot" : ""}"><span class="nav-tab-label">进度</span></button>
            <button type="button" data-tab="history" aria-label="历史" class="${tab === "history" ? "active" : ""}"><span class="nav-tab-label">记录</span></button>
          </nav>
          <div class="header-drag-space" aria-hidden="true"></div>
          <div class="header-actions">
          <section class="my-orders" id="my_orders" aria-label="未核销预约"></section>
          <nav class="nav-tabs nav-utility" aria-label="应用"><button type="button" data-tab="settings" aria-label="设置" class="header-settings ${tab === "settings" ? "active" : ""}">${SVG.tabSettings}</button></nav>
          <div class="titlebar-right">${wins}</div>
          </div>
        </header>
        <div class="shell-main">
          <main class="main">
            <div class="content" id="panel"></div>
          </main>
        </div>
      </div>
    </div>
  `;
  chromeBound = false;
  bindChromeOnce();
  updateNavActive();
  mountSettingsUpdateIndicator(app.querySelector<HTMLButtonElement>(".header-settings")!);
  mountMyOrders(app.querySelector<HTMLElement>("#my_orders")!, { popover: true, icon: calendarIcon });
  if (!dateClockStarted) {
    dateClockStarted = true;
    startTargetDateCalibrationClock(() => {
      if (tab === "setup") void showTab("setup");
    });
  }
}

/** 首次挂载壳；之后切 Tab 只换 panel，避免整页重绘卡顿 */
export function render() {
  const end = log.time("chrome", "render_mount");
  const firstMount = !app.querySelector(".shell");
  if (firstMount) {
    mountShell();
  } else {
    updateNavActive();
    updateWinButtons();
    bindChromeOnce();
  }
  end();
  // 冷启动先等日期校准落盘，再画任务页，避免与 mount 末尾 scheduleSave 竞态盖回旧日。
  if (firstMount) {
    void whenBootCalibrationDone().then(() => showTab(tab));
  } else {
    void showTab(tab);
  }
}

export { win };
