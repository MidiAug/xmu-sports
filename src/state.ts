import type { LiveStatus, ShellThemePref, Tab, WechatProbe } from "./types";

export const app = document.querySelector("#app")!;

export let tab: Tab = "setup";
try { if (localStorage.getItem("xmu.booking.mode") === "plans") tab = "plans"; } catch { /* Use single booking. */ }
export let maximized = false;
/** 启动时由 `applyTheme(readThemePref())` 写入 */
export let themePref: ShellThemePref = "system";
/** 侧栏 / 任务栏绿点：挂机等待或HTTP 执行中 */
export let runIndicatorOn = false;
export let liveTickTimer = 0;
export let liveStatusCache: LiveStatus | null = null;
export let liveWxCache: WechatProbe | null = null;
/** 递增以丢弃过期的 get_live_status / probe 回调 */
export let liveRenderGen = 0;
const statusListeners = new Set<(status: LiveStatus) => void>();
export function subscribeLiveStatus(listener: (status: LiveStatus) => void) {
  statusListeners.add(listener);
  return () => { statusListeners.delete(listener); };
}

export function setTab(next: Tab) {
  tab = next;
  if (next === "setup" || next === "plans") {
    try { localStorage.setItem("xmu.booking.mode", next === "plans" ? "plans" : "single"); } catch { /* Keep session navigation. */ }
  }
}

export function setMaximized(v: boolean) {
  maximized = v;
}

export function setThemePref(v: ShellThemePref) {
  themePref = v;
}

export function setRunIndicatorOn(v: boolean) {
  runIndicatorOn = v;
}

export function setLiveTickTimer(v: number) {
  liveTickTimer = v;
}

export function setLiveStatusCache(v: LiveStatus | null) {
  liveStatusCache = v;
  if (v) statusListeners.forEach((listener) => listener(v));
}

export function setLiveWxCache(v: WechatProbe | null) {
  liveWxCache = v;
}

export function bumpLiveRenderGen(): number {
  liveRenderGen += 1;
  return liveRenderGen;
}
