import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LiveStatus } from "./types";
import { app, liveStatusCache, setLiveStatusCache, tab, themePref } from "./state";
import { applyTheme, readThemePref } from "./lib/theme";
import { calibrateSavedTaskTargetDate } from "./lib/calibrate-target-date";
import { attachAppConsole, log } from "./lib/log";
import { ingestLiveStatus, seedRunFeedback } from "./lib/run-feedback";
import { render, syncMaximized, win, openAppUpdates } from "./ui/chrome";
import { showUpdateExit } from "./ui/update-exit";
import { showCloseAsk } from "./ui/close-ask";
import { preloadHistory } from "./pages/history";
import { renderLive } from "./pages/live";
import { renderBooking as renderSetup } from "./pages/booking";
import { askConfirm } from "./ui/confirm-ask";
import { closeShellSelects } from "./ui/select";
import { getVersion } from "@tauri-apps/api/app";
import { startAutoAppUpdate } from "./lib/app-update";

type BootNav = {
  connect_ms: number;
  ttfb_ms: number;
  response_ms: number;
  dom_ms: number;
  transfer_size: number;
};

type BootStamp = {
  html_ms: number;
  href: string;
  nav: BootNav | null;
};

const bootStamp = (window as unknown as { __xmuBoot?: BootStamp }).__xmuBoot;
const moduleEvalMs = Math.round(performance.now());

void attachAppConsole().then(() => {
  log.info("boot", "console_attached");
});
log.info("boot", "ui_start", {
  href: bootStamp?.href ?? location.href,
  html_ms: bootStamp?.html_ms ?? -1,
  module_eval_ms: moduleEvalMs,
  since_html_ms:
    bootStamp && bootStamp.html_ms >= 0
      ? Math.max(0, moduleEvalMs - bootStamp.html_ms)
      : -1,
  nav_connect_ms: bootStamp?.nav?.connect_ms ?? -1,
  nav_ttfb_ms: bootStamp?.nav?.ttfb_ms ?? -1,
  nav_response_ms: bootStamp?.nav?.response_ms ?? -1,
  nav_dom_ms: bootStamp?.nav?.dom_ms ?? -1,
  nav_transfer_size: bootStamp?.nav?.transfer_size ?? -1,
});

applyTheme(readThemePref());
window.matchMedia?.("(prefers-color-scheme: dark)")?.addEventListener("change", () => {
  if (themePref === "system") applyTheme("system");
});
document.addEventListener("click", closeShellSelects);
document.addEventListener("contextmenu", event => event.preventDefault(), { capture: true });
void win.onResized(() => {
  void syncMaximized();
});
void listen<{ armed?: boolean }>("shell-ask-close", (ev) => {
  log.info("shell", "ask_close", { armed: Boolean(ev.payload?.armed) });
  showCloseAsk(Boolean(ev.payload?.armed));
});
void listen("shell-plan-exit", async () => {
  closeShellSelects();
  if (await askConfirm({title:"退出后自动预约将暂停",lead:"长期计划会保留。保持电脑唤醒并留在托盘，才能按时预约。",confirmLabel:"退出应用",cancelLabel:"留在后台"})) await invoke("force_quit_app");
  else await invoke("hide_to_tray");
});
void listen<{plans?:boolean}>("shell-update-exit",event=>{closeShellSelects();showUpdateExit(openAppUpdates,Boolean(event.payload?.plans));});
void listen("arm-changed", () => {
  log.debug("arm", "arm_changed");
  refreshLiveStatusOnly();
  // Armed skipped 0/7 calibrate; catch up after disarm / fire clears arm.
  void calibrateSavedTaskTargetDate().then((changed) => {
    if (changed && tab === "setup") {
      const panel = app.querySelector("#panel");
      if (panel) void renderSetup(panel);
    }
  });
});
void listen("plans-changed", () => { if (tab === "live") refreshLiveStatusOnly(); });
void listen("run-changed", () => {
  log.debug("live", "run_changed");
  refreshLiveStatusOnly();
  void calibrateSavedTaskTargetDate().then((changed) => {
    if (changed && tab === "setup") {
      const panel = app.querySelector("#panel");
      if (panel) void renderSetup(panel);
    }
  });
});

let refreshing = false;
let refreshPending = false;
function refreshLiveStatusOnly() {
  if (refreshing) { refreshPending = true; return; }
  refreshing = true;
  void invoke<LiveStatus>("get_live_status")
    .then((s) => {
      ingestLiveStatus(s);
      if (tab === "live") {
        const panel = app.querySelector("#panel");
        if (panel) void renderLive(panel, { statusOnly: true, status: s });
      }
    })
    .catch((e) => log.warn("arm", "refresh_status_failed", { err: String(e) }))
    .finally(() => { refreshing = false; if (refreshPending) { refreshPending = false; refreshLiveStatusOnly(); } });
}

void invoke<LiveStatus>("get_live_status")
  .then((s) => {
    seedRunFeedback(s);
    setLiveStatusCache(s);
    log.info("boot", "live_status_cached", {
      armed: s.armed,
      running: s.running,
      has_task: Boolean(s.task),
    });
  })
  .catch((e) => log.warn("boot", "get_live_status_failed", { err: String(e) }));
render();

let stopAutoUpdate: (() => void) | undefined;
if (!import.meta.env.DEV) {
  void getVersion().then(version => {
    stopAutoUpdate = startAutoAppUpdate({ version, enabled: true, isBusy: () =>
      !liveStatusCache || liveStatusCache.armed || liveStatusCache.running || Boolean(liveStatusCache.stopping) || Boolean(liveStatusCache.operation),
    });
  }).catch(error => log.debug("update", "auto_check_unavailable", { err: String(error) }));
}
import.meta.hot?.dispose(() => stopAutoUpdate?.());

// Warm the read-only history cache after the first paint, never block app startup.
const warmHistory = () => void preloadHistory().catch(error => log.debug("history", "preload_failed", {err:String(error)}));
if ("requestIdleCallback" in window) window.requestIdleCallback(warmHistory, {timeout:1500});
else setTimeout(warmHistory, 400);
