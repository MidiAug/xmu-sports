import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { LiveStatus } from "./types";
import { app, setLiveStatusCache, tab, themePref } from "./state";
import { applyTheme, readThemePref } from "./lib/theme";
import { calibrateSavedTaskTargetDate } from "./lib/calibrate-target-date";
import { attachAppConsole, log } from "./lib/log";
import { ingestLiveStatus, seedRunFeedback } from "./lib/run-feedback";
import { render, syncMaximized, win } from "./ui/chrome";
import { showCloseAsk } from "./ui/close-ask";
import { renderLive } from "./pages/live";
import { renderSetup } from "./pages/setup";
import { closeShellSelects } from "./ui/select";

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
void win.onResized(() => {
  void syncMaximized();
});
void listen<{ armed?: boolean }>("shell-ask-close", (ev) => {
  log.info("shell", "ask_close", { armed: Boolean(ev.payload?.armed) });
  showCloseAsk(Boolean(ev.payload?.armed));
});
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
