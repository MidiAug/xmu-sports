import { invoke } from "@tauri-apps/api/core";
import type { BookingTask } from "../types";
import { createSaveQueue } from "./save-queue";
import { liveStatusCache, setLiveStatusCache } from "../state";
let revision: number | undefined;
let loadedId: string | undefined;
const queue = createSaveQueue<BookingTask>(async (draft) => {
  const task = { ...draft, revision: loadedId === draft.task_id ? revision ?? draft.revision ?? 0 : draft.revision ?? 0 };
  const saved = await invoke<BookingTask>("save_task", { task });
  revision = saved.revision ?? 0;
  loadedId = saved.task_id;
  if (liveStatusCache && !liveStatusCache.armed && !liveStatusCache.running) {
    setLiveStatusCache({ ...liveStatusCache, task: saved });
  }
});
export async function readSavedTask(): Promise<BookingTask | null> {
  await queue.flush();
  const task = await invoke<BookingTask | null>("read_task_raw");
  revision = task?.revision ?? 0;
  loadedId = task?.task_id;
  return task;
}
export function saveTaskDraft(task: BookingTask) { queue.enqueue(structuredClone(task)); }
export const flushTask = queue.flush;
export const subscribeTaskSave = queue.subscribe;
export const discardTaskDraft = queue.discard;
