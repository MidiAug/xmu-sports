import { invoke } from "@tauri-apps/api/core";
import type { BookingTask } from "../types";
import { log } from "./log";
import { plannedFireAt, defaultTargetDateISO, fireHmFromIso, isSelectableTargetDate, localDateISO } from "./time";
import { readSavedTask, saveTaskDraft, flushTask } from "./task-store";

/** Preserve a valid absolute target; only expired/out-of-range goals use the default. */
export function withDefaultTargetDate(task: BookingTask, now = new Date()): BookingTask {
  const target_date = isSelectableTargetDate(task.target_date, now) ? task.target_date : defaultTargetDateISO(now);
  const delay = task.fire_delay_seconds ?? 0;
  const fireHm = fireHmFromIso(task.fire_at, delay);
  const fire_at = plannedFireAt(target_date, fireHm, delay, now);
  if (task.target_date === target_date && task.fire_at === fire_at) return task;
  return {
    ...task,
    target_date,
    fire_at,
  };
}

/** ms until the sooner of next local 00:00 or 07:00. */
export function msUntilNextDateBoundary(now = new Date()): number {
  const times: number[] = [];
  for (const hour of [0, 7]) {
    const d = new Date(`${localDateISO(now)}T${String(hour).padStart(2, "0")}:00:00+08:00`);
    if (d.getTime() <= now.getTime()) d.setUTCDate(d.getUTCDate() + 1);
    times.push(d.getTime());
  }
  return Math.max(1000, Math.min(...times) - now.getTime());
}

/** Include today: midnight changes the three choices even if the target stays the same. */
let lastCalibratedWindow: string | null = null;

/**
 * Refresh execution time and expired goals on cold start and 00:00 / 07:00 boundaries.
 * Valid absolute dates survive boundaries, navigation and restart.
 * Armed / running freezes the shot — skip and do not mark calibrated so disarm can catch up.
 * Returns true if disk was updated.
 */
export async function calibrateSavedTaskTargetDate(now = new Date()): Promise<boolean> {
  const desired = defaultTargetDateISO(now);
  const windowKey = `${localDateISO(now)}:${desired}`;
  if (lastCalibratedWindow === windowKey) return false;

  try {
    const st = await invoke<{ armed?: boolean; running?: boolean }>("get_live_status");
    if (st?.armed || st?.running) {
      log.debug("setup", "calibrate_target_date_skipped", {
        reason: "armed_or_running",
        desired,
        armed: Boolean(st.armed),
        running: Boolean(st.running),
      });
      return false;
    }
  } catch {
    return false;
  }

  let raw: BookingTask | null = null;
  try {
    raw = await readSavedTask();
  } catch {
    return false;
  }
  if (!raw || typeof raw !== "object" || !("task_id" in raw)) {
    lastCalibratedWindow = windowKey;
    return false;
  }
  const next = withDefaultTargetDate(raw, now);
  if (next.target_date === raw.target_date && next.fire_at === raw.fire_at) {
    lastCalibratedWindow = windowKey;
    return false;
  }
  try {
    saveTaskDraft(next);
    await flushTask();
    lastCalibratedWindow = windowKey;
    log.debug("setup", "calibrate_target_date", {
      from: raw.target_date,
      to: next.target_date,
    });
    return true;
  } catch (e) {
    log.warn("setup", "calibrate_target_date_failed", { err: String(e) });
    return false;
  }
}

let boundaryTimer = 0;
let clockStarted = false;
let bootCalibration: Promise<void> | null = null;

/** Resolves after the cold-start calibrate (safe to await before first setup paint). */
export function whenBootCalibrationDone(): Promise<void> {
  return bootCalibration ?? Promise.resolve();
}

/**
 * Schedule calibrate at next 0:00 / 7:00; visibility only catches a missed boundary
 * (same default window → skip, keeps manual date).
 * `onCalibrated` when a window was applied (e.g. refresh setup's relative choices).
 */
export function startTargetDateCalibrationClock(onCalibrated?: () => void): void {
  if (clockStarted) return;
  clockStarted = true;

  const run = async () => {
    const previousWindow = lastCalibratedWindow;
    const changed = await calibrateSavedTaskTargetDate();
    // Refresh the three relative date choices even without a saved task, or when
    // midnight keeps the same absolute target. Skipped locked tasks don't repaint.
    if (changed || previousWindow !== lastCalibratedWindow) onCalibrated?.();
  };

  const arm = () => {
    window.clearTimeout(boundaryTimer);
    boundaryTimer = window.setTimeout(() => {
      void run().finally(arm);
    }, msUntilNextDateBoundary());
  };

  arm();
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") void run();
  });
  bootCalibration = run().then(() => undefined);
}
