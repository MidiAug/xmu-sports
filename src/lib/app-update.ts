import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type ReleaseInfo = { status: "available" | "current" | "unpublished"; version: string; notes: string; size: number };
type UpdateView = { phase: "idle" | "checking" | "ready" | "downloading" | "downloaded" | "error"; release?: ReleaseInfo; error?: string; progress: number; path?: string };
// Shared across settings subpages; navigation neither repeats nor cancels a request.
let view: UpdateView = { phase: "idle", progress: 0 };
const subscribers = new Set<(view: UpdateView) => void>();
function publish(patch: Partial<UpdateView>) { view = { ...view, ...patch }; for (const fn of subscribers) fn(view); }
export function watchAppUpdate(fn: (view: UpdateView) => void): () => void { subscribers.add(fn); fn(view); return () => { subscribers.delete(fn); }; }
export async function checkAppUpdate() {
  if (view.phase === "checking" || view.phase === "downloading") return;
  publish({ phase: "checking", error: undefined, path: undefined });
  try { publish({ phase: "ready", release: await invoke<ReleaseInfo>("check_app_update") }); }
  catch (e) { publish({ phase: "error", release: undefined, error: String(e) }); }
}
export async function downloadAppUpdate() {
  if (view.phase === "checking" || view.phase === "downloading" || view.release?.status !== "available") return;
  publish({ phase: "downloading", progress: 0, error: undefined });
  let unlisten: (() => void) | undefined;
  try {
    unlisten = await listen<{ received: number; total: number }>("app-update-progress", ({ payload }) => {
      if (payload.total > 0) publish({ progress: Math.min(100, Math.round(payload.received * 100 / payload.total)) });
    });
    const path = await invoke<string>("download_app_update");
    publish({ phase: "downloaded", progress: 100, path });
  } catch (e) { publish({ phase: "error", error: String(e) }); }
  finally { unlisten?.(); }
}
