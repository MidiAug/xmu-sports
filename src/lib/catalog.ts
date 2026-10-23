import catalog from "../catalog/venues.json";
import type { Sport, TimeSlot, Venue } from "../types";

export const sports = (catalog as { sports: Sport[] }).sports;

export function isSportEnabled(s: Sport): boolean {
  return s.enabled !== false && s.venues.length > 0;
}

export function enabledSports(): Sport[] {
  return sports.filter(isSportEnabled);
}

export function defaultFavoriteSportIds(): string[] {
  return enabledSports().map((s) => s.id);
}

/** Keep catalog order; drop unknown / disabled ids; empty → all enabled. */
export function resolveFavoriteSportIds(ids: string[] | null | undefined): string[] {
  const enabled = enabledSports();
  if (!ids || ids.length === 0) return enabled.map((s) => s.id);
  const want = new Set(ids);
  const hit = enabled.filter((s) => want.has(s.id)).map((s) => s.id);
  return hit.length > 0 ? hit : enabled.map((s) => s.id);
}

export function favoriteSports(ids: string[] | null | undefined): Sport[] {
  const resolved = new Set(resolveFavoriteSportIds(ids));
  return enabledSports().filter((s) => resolved.has(s.id));
}

export function findSport(ocrOrId: string): Sport | undefined {
  return sports.find((s) => s.ocr_label === ocrOrId || s.id === ocrOrId || s.label === ocrOrId);
}

export function findVenue(sport: Sport, ocrOrId: string): Venue | undefined {
  return sport.venues.find(
    (v) => v.ocr_label === ocrOrId || v.id === ocrOrId || v.label === ocrOrId,
  );
}

export function slotLabel(slot: TimeSlot): string {
  return `${slot.start}–${slot.end}`;
}
