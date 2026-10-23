import type { CloseMode, ShellPrefs } from "../types";

export function closeModeFromPrefs(p: ShellPrefs): CloseMode {
  if (!p.close_pref_set) return "ask";
  return p.close_to_tray ? "tray" : "quit";
}

export function prefsFromCloseMode(
  mode: CloseMode,
  rememberUsed: boolean,
  favoriteSportIds?: string[],
  venueSlotPrefs?: Record<string, string>,
): ShellPrefs {
  const favorites = favoriteSportIds ?? [];
  const slots = venueSlotPrefs ?? {};
  if (mode === "ask")
    return {
      close_to_tray: true,
      close_pref_set: false,
      close_remember_default_used: rememberUsed,
      favorite_sport_ids: favorites,
      venue_slot_prefs: slots,
    };
  if (mode === "tray")
    return {
      close_to_tray: true,
      close_pref_set: true,
      close_remember_default_used: rememberUsed,
      favorite_sport_ids: favorites,
      venue_slot_prefs: slots,
    };
  return {
    close_to_tray: false,
    close_pref_set: true,
    close_remember_default_used: rememberUsed,
    favorite_sport_ids: favorites,
    venue_slot_prefs: slots,
  };
}

/** Merge close-mode fields onto existing prefs without dropping favorites / slot prefs. */
export function withCloseMode(prefs: ShellPrefs, mode: CloseMode): ShellPrefs {
  return {
    ...prefsFromCloseMode(
      mode,
      prefs.close_remember_default_used,
      prefs.favorite_sport_ids,
      prefs.venue_slot_prefs,
    ),
    detailed_diagnostics: prefs.detailed_diagnostics,
  };
}
