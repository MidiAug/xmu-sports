import { invoke } from "@tauri-apps/api/core";
import { app, liveTickTimer, runIndicatorOn, setLiveTickTimer, setRunIndicatorOn } from "../state";

export function setRunIndicator(on: boolean) {
  if (on === runIndicatorOn) return;
  setRunIndicatorOn(on);
  app.querySelectorAll('[data-tab="live"]').forEach((btn) => {
    btn.classList.toggle("has-run-dot", on);
  });
  void invoke("set_run_badge", { on }).catch(() => {});
}

export function clearLiveTick() {
  if (liveTickTimer) {
    window.clearInterval(liveTickTimer);
    setLiveTickTimer(0);
  }
}
