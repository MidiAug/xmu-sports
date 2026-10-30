import { invoke } from "@tauri-apps/api/core";
import { app, liveTickTimer, runIndicatorOn, setLiveTickTimer, setRunIndicatorOn } from "../state";

let taskActive = false;
let planUpcoming = false;
function paintIndicator() {
  const on = taskActive || planUpcoming;
  if (on === runIndicatorOn) return;
  setRunIndicatorOn(on);
  app.querySelectorAll('[data-tab="live"]').forEach(btn => btn.classList.toggle("has-run-dot", on));
}

export function setRunIndicator(on: boolean) {
  if (on !== taskActive) void invoke("set_run_badge", { on }).catch(() => {});
  taskActive = on;
  paintIndicator();
}

export function setPlanRunIndicator(on: boolean) {
  planUpcoming = on;
  paintIndicator();
}

export function clearLiveTick() {
  if (liveTickTimer) {
    window.clearInterval(liveTickTimer);
    setLiveTickTimer(0);
  }
}
