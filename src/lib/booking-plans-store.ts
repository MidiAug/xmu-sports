import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { hasUpcomingPlan, type BookingPlansView } from "./booking-plans";
import { setPlanRunIndicator } from "./run-indicator";
import { addDaysISO, localDateISO } from "./time";

export type PlansSnapshot = { view?: BookingPlansView; error?: unknown };

/** Plan events invalidate one application snapshot; page renders never poll the host. */
export function createBookingPlansStore(fetcher: () => Promise<BookingPlansView>, now = Date.now) {
  let state: PlansSnapshot = {};
  let dirty = true;
  let pending: Promise<BookingPlansView> | undefined;
  let readDay: string | undefined;
  const listeners = new Set<(snapshot: PlansSnapshot) => void>();
  const notify = () => listeners.forEach(listener => listener(state));
  function read(force = false): Promise<BookingPlansView> {
    const today = localDateISO(new Date(now()));
    if (pending) return pending;
    if (force || readDay !== today) dirty = true;
    if (!dirty && state.view && !state.error) return Promise.resolve(state.view);
    dirty = false;
    readDay = today;
    pending = fetcher().then(view => {
      state = { view }; notify(); return view;
    }, error => {
      state = { view: state.view, error }; notify(); throw error;
    }).finally(() => {
      pending = undefined;
      // A host event can arrive during a read, including save's plans-changed event.
      if (dirty && listeners.size) void read().catch(() => {});
    });
    return pending;
  }
  return {
    read,
    snapshot: () => state,
    invalidate() { dirty = true; if (listeners.size) void read().catch(() => {}); },
    subscribe(listener: (snapshot: PlansSnapshot) => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
  };
}

export const bookingPlans = createBookingPlansStore(() => invoke<BookingPlansView>("get_booking_plans"));

export async function startBookingPlansUpdates() {
  const stops = await Promise.all(["plans-changed", "plans-health"].map(event => listen(event, () => bookingPlans.invalidate())));
  const stopIndicator = bookingPlans.subscribe(snapshot => {
    if (!snapshot.error && snapshot.view) {
      const now = new Date(); const today = localDateISO(now);
      setPlanRunIndicator(hasUpcomingPlan(snapshot.view, today, addDaysISO(today,1), now.getTime()));
    }
  });
  void bookingPlans.read().catch(() => {});
  let day: string;
  let timer: number;
  const armMidnight = () => {
    window.clearTimeout(timer);
    day = localDateISO(new Date());
    const midnight = Date.parse(`${addDaysISO(day, 1)}T00:00:00+08:00`);
    timer = window.setTimeout(() => { bookingPlans.invalidate(); armMidnight(); }, midnight - Date.now());
  };
  const onVisible = () => {
    if (document.visibilityState === "visible" && localDateISO(new Date()) !== day) {
      bookingPlans.invalidate(); armMidnight();
    }
  };
  // Paused plans have no health events, but their date previews must still roll over.
  armMidnight();
  document.addEventListener("visibilitychange", onVisible);
  return () => { stopIndicator(); stops.forEach(stop => stop()); window.clearTimeout(timer); document.removeEventListener("visibilitychange", onVisible); };
}
