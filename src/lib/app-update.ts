import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ReleaseInfo = { status: "available" | "current" | "unpublished"; version: string; notes: string; size: number };
type UpdateView = { phase: "idle" | "checking" | "ready" | "downloading" | "downloaded" | "installing" | "error"; release?: ReleaseInfo; error?: string; failedAt?: "check" | "download" | "install"; progress: number; path?: string };
// Shared across settings subpages; navigation neither repeats nor cancels a request.
let view: UpdateView = { phase: "idle", progress: 0 };
const subscribers = new Set<(view: UpdateView) => void>();
function publish(patch: Partial<UpdateView>) { view = { ...view, ...patch }; for (const fn of subscribers) fn(view); }
export function watchAppUpdate(fn: (view: UpdateView) => void): () => void { subscribers.add(fn); fn(view); return () => { subscribers.delete(fn); }; }
export async function checkAppUpdate() {
  if (["checking", "downloading", "installing"].includes(view.phase)) return;
  publish({ phase: "checking", error: undefined, path: undefined });
  try { publish({ phase: "ready", release: await invoke<ReleaseInfo>("check_app_update") }); }
  catch (e) { publish({ phase: "error", release: undefined, failedAt: "check", error: String(e) }); }
}
export async function downloadAppUpdate() {
  if (["checking", "downloading", "installing"].includes(view.phase) || view.release?.status !== "available") return;
  publish({ phase: "downloading", progress: 0, error: undefined });
  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ received: number; total: number }>("app-update-progress", ({ payload }) => {
      if (payload.total > 0) publish({ progress: Math.min(100, Math.round(payload.received * 100 / payload.total)) });
    });
    const path = await invoke<string>("download_app_update");
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
