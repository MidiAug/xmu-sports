import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ReleaseInfo = { status: "available" | "current" | "unpublished"; version: string; notes: string; size: number };
type UpdateView = { phase: "idle" | "checking" | "ready" | "downloading" | "downloaded" | "installing" | "error"; release?: ReleaseInfo; error?: string; failedAt?: "check" | "download" | "install"; progress: number; path?: string };
// Shared across settings subpages; navigation neither repeats nor cancels a request.
let view: UpdateView = { phase: "idle", progress: 0 };
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;
type CheckRecord = { nextCheckAt: number; release?: ReleaseInfo };
let record: CheckRecord = { nextCheckAt: 0 };
let recordKey: string | undefined;
let hostCandidateReady = false;
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
  if (!updateDownloadRoutes.some(route=>route.value===value) || ["downloading","installing"].includes(view.phase)) return false;
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
      if (view.phase === "idle") publish({ phase: "ready", release });
    }
  } catch { /* Corrupt or unavailable storage must not stop startup. */ }
}
const subscribers = new Set<(view: UpdateView) => void>();
function publish(patch: Partial<UpdateView>) { view = { ...view, ...patch }; for (const fn of subscribers) fn(view); }
export function watchAppUpdate(fn: (view: UpdateView) => void): () => void { subscribers.add(fn); fn(view); return () => { subscribers.delete(fn); }; }
export async function checkAppUpdate() {
  if (["checking", "downloading", "installing"].includes(view.phase)) return;
  publish({ phase: "checking", error: undefined, path: undefined });
  hostCandidateReady = false;
  // Also covers a process exit during a request, before a result can be saved.
  saveRecord({ nextCheckAt: Date.now() + HOUR });
  try {
    const release = await invoke<ReleaseInfo>("check_app_update");
    hostCandidateReady = release.status === "available";
    saveRecord({ nextCheckAt: Date.now() + DAY, release });
    publish({ phase: "ready", release });
  }
  catch (e) {
    saveRecord({ nextCheckAt: Date.now() + HOUR });
    publish({ phase: "error", release: undefined, failedAt: "check", error: String(e) });
  }
}
export async function downloadAppUpdate() {
  if (["checking", "downloading", "installing"].includes(view.phase) || view.release?.status !== "available") return;
  if (!hostCandidateReady) {
    await checkAppUpdate();
    if (!hostCandidateReady) return;
  }
  publish({ phase: "downloading", progress: 0, error: undefined });
  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ received: number; total: number }>("app-update-progress", ({ payload }) => {
      if (payload.total > 0) publish({ progress: Math.min(100, Math.round(payload.received * 100 / payload.total)) });
    });
    const path = await invoke<string>("download_app_update", { mirror: updateDownloadRoute() });
    publish({ phase: "downloaded", progress: 100, path });
  } catch (e) { publish({ phase: "error", failedAt: "download", error: String(e) }); }
  finally { unlisten?.(); }
}
export async function installAppUpdate() {
  if (view.phase !== "downloaded" && !(view.phase === "error" && view.failedAt === "install")) return;
  publish({ phase: "installing", error: undefined });
  try { await invoke("install_app_update"); }
  catch (e) { const error = String(e); publish({ phase: "error", failedAt: error.includes("重新下载") ? "download" : "install", error, path: error.includes("重新下载") ? undefined : view.path }); }
}
export function performAppUpdate() {
  if (view.phase === "downloaded" || (view.phase === "error" && view.failedAt === "install")) return installAppUpdate();
  if (view.release?.status === "available" && (view.phase === "ready" || (view.phase === "error" && view.failedAt === "download"))) return downloadAppUpdate();
  return checkAppUpdate();
}

/** App-scoped scheduler: no page navigation or automatic download/install. */
export function startAutoAppUpdate(options: { version: string; enabled: boolean; isBusy: () => boolean }) {
  if (!options.enabled) return () => {};
  restoreRecord(options.version);
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;
  const notBefore = Date.now() + 30_000;
  const protectedState = () => view.release?.status === "available"
    || ["checking", "downloading", "downloaded", "installing"].includes(view.phase)
    || (view.phase === "error" && view.failedAt !== "check");
  const schedule = (minimum = 0) => {
    if (stopped) return;
    clearTimeout(timer);
    const delay = Math.max(minimum, notBefore - Date.now(), record.nextCheckAt - Date.now(), 0);
    timer = setTimeout(() => { void tick(); }, delay);
  };
  const tick = async () => {
    if (stopped) return;
    if (Date.now() < notBefore || Date.now() < record.nextCheckAt) { schedule(); return; }
    if (protectedState()) { schedule(view.phase === "checking" ? 60_000 : DAY); return; }
    if (options.isBusy()) { schedule(60_000); return; }
    await checkAppUpdate();
    schedule();
  };
  const wake = () => schedule();
  window.addEventListener("focus", wake);
  window.addEventListener("online", wake);
  document.addEventListener("visibilitychange", wake);
  schedule();
  return () => {
    stopped = true;
    clearTimeout(timer);
    window.removeEventListener("focus", wake);
    window.removeEventListener("online", wake);
    document.removeEventListener("visibilitychange", wake);
  };
}
