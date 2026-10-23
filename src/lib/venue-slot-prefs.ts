import type { TimeSlot } from "../types";

export function slotPrefKey(start: string, end: string): string {
  return `${start}|${end}`;
}

/** Prefer saved "start|end" if still in catalog; else first slot. */
export function resolvePreferredSlot(
  slots: TimeSlot[],
  prefer?: string | null,
): string {
  const first = slotPrefKey(slots[0].start, slots[0].end);
  if (prefer && slots.some((t) => slotPrefKey(t.start, t.end) === prefer)) {
    return prefer;
  }
  return first;
}
