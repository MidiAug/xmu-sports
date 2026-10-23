import { invoke } from "@tauri-apps/api/core";
import type { ShellPrefs } from "../types";
let tail = Promise.resolve();
/** Merge at the head of the queue so independent settings cannot clobber one another. */
function changePrefs(update: (current: ShellPrefs) => ShellPrefs): Promise<ShellPrefs> {
  const next = tail.then(async () => {
    const current = await invoke<ShellPrefs>("get_shell_prefs");
    const prefs = update(current);
    await invoke("save_shell_prefs", { prefs });
    return prefs;
  });
  tail = next.then(() => {}, () => {});
  return next;
}
export function updatePrefs(patch: Partial<ShellPrefs>): Promise<ShellPrefs> {
  return changePrefs(current => ({ ...current, ...patch }));
}
export function updateVenueSlotPreference(venue: string, slot: string | null) {
  return changePrefs(current => {
    const slots = { ...current.venue_slot_prefs };
    if (slot === null) delete slots[venue]; else slots[venue] = slot;
    return { ...current, venue_slot_prefs: slots };
  });
}
