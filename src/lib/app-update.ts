import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ReleaseInfo = { status: "available" | "current" | "unpublished"; version: string; notes: string; size: number };
type UpdateView = { phase: "idle" | "checking" | "ready" | "downloading" | "cancelling" | "downloaded" | "installing" | "error"; release?: ReleaseInfo; error?: string; failedAt?: "check" | "download" | "install"; progress: number; path?: string; paused?:boolean; cancelled?:boolean };
// Shared across settings subpages; navigation neither repeats nor cancels a request.
let view: UpdateView = { phase: "idle", progress: 0 };
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;
type CheckRecord = { nextCheckAt: number; release?: ReleaseInfo };
let record: CheckRecord = { nextCheckAt: 0 };
let recordKey: string | undefined;
let hostCandidateReady = false;
const cancelledVersionKey="xmu.appUpdate.cancelledVersion.v1";
let cancelledVersion="";
try { cancelledVersion=localStorage.getItem(cancelledVersionKey) ?? ""; } catch { /* Memory fallback. */ }
let downloadId:string|undefined;
let cancelRequested=false;
let downloadStarted=false;
function suppressDownload(version:string) {
  cancelledVersion=version;
  try {localStorage.setItem(cancelledVersionKey,version);} catch { /* Keep the user's choice in memory. */ }
}
const autoDownloadKey="xmu.appUpdate.autoDownload.v1";
export function autoDownloadUpdate():boolean {
  try { return localStorage.getItem(autoDownloadKey)!=="false"; } catch { return true; }
}
export function setAutoDownloadUpdate(enabled:boolean):boolean {
  try { localStorage.setItem(autoDownloadKey,String(enabled)); if(!enabled && view.paused)publish({paused:false}); window.dispatchEvent(new Event("app-update-preference")); return true; } catch { return false; }
}
export const updateDownloadRoutes = [
  { value: "official", label: "GitHub 直连" },
  { value: "gh-proxy", label: "gh-proxy.com" },
  { value: "ghproxy", label: "ghproxy.net" },
];
const routeKey = "xmu.appUpdate.downloadRoute.v1";
export function updateDownloadRoute():string {
  try {
    const value=localStorage.getItem(routeKey);
    return updateDownloadRoutes.some(route=>route.value===value) ? value! : "official";
  } catch { return "official"; }
}
export function setUpdateDownloadRoute(value:string):boolean {
  if (!updateDownloadRoutes.some(route=>route.value===value) || ["checking","downloading","cancelling","installing"].includes(view.phase)) return false;
  try { localStorage.setItem(routeKey,value); return true; } catch { return false; }
}
function saveRecord(next: CheckRecord) {
  record = next;
  try { if (recordKey) localStorage.setItem(recordKey, JSON.stringify(record)); } catch { /* Keep in-memory cooldown when storage is unavailable. */ }
}
function restoreRecord(version: string) {
  const key = `xmu.appUpdate.v1.${version}`;
  if (recordKey === key) return;
  recordKey = key;
  try {
    const cached = JSON.parse(localStorage.getItem(key) ?? "null");
    const now = Date.now();
    if (!cached || !Number.isFinite(cached.nextCheckAt) || cached.nextCheckAt < 0 || cached.nextCheckAt > now + DAY) return;
    record = { nextCheckAt: cached.nextCheckAt };
    const release = cached.release;
    if (release && ["available", "current", "unpublished"].includes(release.status)
      && typeof release.version === "string" && release.version.length <= 64
      && typeof release.notes === "string" && release.notes.length <= 64000
      && Number.isFinite(release.size) && release.size >= 0 && release.size <= 512 * 1024 * 1024) {
      record.release = release;
      if (view.phase === "idle") publish({ phase: "ready", release, cancelled:cancelledVersion===release.version });
    }
  } catch { /* Corrupt or unavailable storage must not stop startup. */ }
}
const subscribers = new Set<(view: UpdateView) => void>();
function publish(patch: Partial<UpdateView>) { view = { ...view, ...patch }; for (const fn of subscribers) fn(view); }
export function watchAppUpdate(fn: (view: UpdateView) => void): () => void { subscribers.add(fn); fn(view); return () => { subscribers.delete(fn); }; }
export async function checkAppUpdate() {
  if (["checking", "downloading", "cancelling", "installing"].includes(view.phase)) return;
  publish({ phase: "checking", error: undefined, path: undefined, paused:false });
  hostCandidateReady = false;
  // Also covers a process exit during a request, before a result can be saved.
  saveRecord({ nextCheckAt: Date.now() + HOUR });
  try {
    const release = await invoke<ReleaseInfo>("check_app_update");
    hostCandidateReady = release.status === "available";
    saveRecord({ nextCheckAt: Date.now() + DAY, release });
    publish({ phase: "ready", release, cancelled:cancelledVersion===release.version });
  }
  catch (e) {
    saveRecord({ nextCheckAt: Date.now() + HOUR });
    publish({ phase: "error", release: undefined, failedAt: "check", error: String(e) });
  }
}
export async function downloadAppUpdate(automatic=false) {
  if (["checking", "downloading", "cancelling", "installing"].includes(view.phase) || view.release?.status !== "available") return;
  if (automatic && cancelledVersion===view.release.version) return;
  if (!hostCandidateReady) {
    await checkAppUpdate();
    if (!hostCandidateReady) return;
  }
  const id=crypto.randomUUID();downloadId=id;cancelRequested=false;downloadStarted=false;
  if(!automatic && cancelledVersion===view.release.version) suppressDownload("");
  publish({ phase: "downloading", progress: 0, error: undefined, path:undefined, paused:false, cancelled:false });
  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ received: number; total: number; download_id:string }>("app-update-progress", ({ payload }) => {
      if (downloadId===id && view.phase==="downloading" && payload.download_id===id && payload.total > 0) publish({ progress: Math.min(100, Math.round(payload.received * 100 / payload.total)) });
    });
    if(cancelRequested) {publish({phase:"ready",progress:0,cancelled:true});return;}
    downloadStarted=true;
    const path = await invoke<string>("download_app_update", { mirror: updateDownloadRoute(), automatic, downloadId:id });
    publish({ phase: "downloaded", progress: 100, path });
  } catch (e) {
    if(cancelRequested || String(e).includes("UPDATE_CANCELLED")) publish({phase:"ready",progress:0,error:undefined,path:undefined,cancelled:true});
    else if(automatic && String(e).includes("UPDATE_PAUSED")) publish({phase:"ready",progress:0,error:undefined,paused:true});
    else publish({ phase: "error", failedAt: "download", error: String(e) });
  }
  finally { unlisten?.();if(downloadId===id) {downloadId=undefined;downloadStarted=false;} }
}
export async function cancelAppUpdateDownload() {
  if(view.phase!=="downloading" || !downloadId) return;
  const id=downloadId;cancelRequested=true;suppressDownload(view.release?.version ?? "");
  publish({phase:"cancelling",error:undefined});
  if(!downloadStarted) return; // The listener setup will finish without starting a request.
  try {await invoke("cancel_app_update_download",{downloadId:id});}
  catch(e) {
    if(downloadId===id && ["cancelling"].includes(view.phase)) {
      cancelRequested=false;publish({phase:"downloading",error:`取消请求失败，请再次点击取消：${String(e)}`});
    }
  }
}
export async function installAppUpdate() {
  if (view.phase !== "downloaded" && !(view.phase === "error" && view.failedAt === "install")) return;
  publish({ phase: "installing", error: undefined });
  try { await invoke("install_app_update"); }
  catch (e) {
    const message = String(e);
    const redownload = message.startsWith("UPDATE_PACKAGE_INVALID:");
    publish({ phase: "error", failedAt: redownload ? "download" : "install",
      error: redownload ? message.slice("UPDATE_PACKAGE_INVALID:".length).trim() : message,
      path: redownload ? undefined : view.path });
  }
}
export function performAppUpdate() {
  if (view.phase === "downloaded" || (view.phase === "error" && view.failedAt === "install")) return installAppUpdate();
  if (view.release?.status === "available" && (view.phase === "ready" || (view.phase === "error" && view.failedAt === "download"))) return downloadAppUpdate();
  return checkAppUpdate();
}

/** App-scoped checking and optional download; installation always requires a click. */
export function startAutoAppUpdate(options: { version: string; enabled: boolean; isBusy: () => boolean }) {
  if (!options.enabled) return () => {};
  restoreRecord(options.version);
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;
  const notBefore = Date.now() + 30_000;
  let downloadRetryAt=0;
  const wantsDownload=()=>autoDownloadUpdate() && view.release?.status==="available"
    && cancelledVersion!==view.release.version
    && (view.phase==="ready" || view.phase==="error" && view.failedAt==="download");
  const protectedState = () => view.release?.status === "available"
    || ["checking", "downloading", "cancelling", "downloaded", "installing"].includes(view.phase)
    || (view.phase === "error" && view.failedAt !== "check");
  const schedule = (minimum = 0) => {
    if (stopped) return;
    clearTimeout(timer);
    const delay = Math.max(minimum, notBefore - Date.now(), (wantsDownload() ? downloadRetryAt : record.nextCheckAt) - Date.now(), 0);
    timer = setTimeout(() => { void tick(); }, delay);
  };
  const tick = async () => {
    if (stopped) return;
    if (Date.now()>=notBefore && wantsDownload()) {
      if(options.isBusy()) {schedule(60_000);return;}
      if(Date.now()<downloadRetryAt) {schedule();return;}
      await downloadAppUpdate(true);
      downloadRetryAt=Date.now()+(view.phase==="ready" ? 60_000 : HOUR);
      schedule();return;
    }
    if (Date.now() < notBefore || Date.now() < record.nextCheckAt) { schedule(); return; }
    if (protectedState()) { schedule(view.phase === "checking" ? 60_000 : DAY); return; }
    if (options.isBusy()) { schedule(60_000); return; }
    await checkAppUpdate();
    schedule();
  };
  const wake = () => schedule();
  const preferenceChanged=()=>{downloadRetryAt=0;schedule();};
  const unwatch=watchAppUpdate(wake);
  window.addEventListener("focus", wake);
  window.addEventListener("online", wake);
  window.addEventListener("app-update-preference",preferenceChanged);
  document.addEventListener("visibilitychange", wake);
  schedule();
  return () => {
    stopped = true;
    unwatch();
    clearTimeout(timer);
    window.removeEventListener("focus", wake);
    window.removeEventListener("online", wake);
    window.removeEventListener("app-update-preference",preferenceChanged);
    document.removeEventListener("visibilitychange", wake);
  };
}
