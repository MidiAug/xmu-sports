import type { RunEvent } from "../types";
import { outcomeFromEvent, type RunOutcome } from "./run-outcome";

export type RunSummary = {
  taskId: string;
  runId?: string | null;
  events: RunEvent[];
  /** null = 未闭合 */
  outcome: RunOutcome | null;
  startedAt: string;
  endedAt: string;
};

/** Successful checks are informational history, rather than booked orders. */
export function historyOutcomeKind(run: RunSummary): RunOutcome["kind"] {
  return ["http_auth_ready", "http_prepared"].includes(run.events.at(-1)?.result ?? "")
    ? "info" : run.outcome?.kind ?? "info";
}

function isTerminalAction(action: string): boolean {
  return (
    action === "done" ||
    action === "fail" ||
    action === "cancelled" ||
    action.endsWith("_failed")
  );
}

function isStartAction(action: string): boolean {
  return action === "spawn" || action === "started";
}

function incompleteOutcome(ev: RunEvent): RunOutcome {
  return {
    kind: "info",
    code: "incomplete",
    title: "未完成",
    detail: "这次运行没有正常结束",
    pageId: ev.page_id ?? null,
    ts: ev.ts ?? null,
  };
}

function toSummary(events: RunEvent[], closed: boolean): RunSummary | null {
  if (!events.length) return null;
  const last = events[events.length - 1];
  const first = events[0];
  let outcome: RunOutcome | null = null;
  if (closed) {
    outcome = outcomeFromEvent(last);
    if (!outcome) {
      outcome = incompleteOutcome(last);
    }
  } else {
    outcome = incompleteOutcome(last);
  }
  return {
    taskId: last.task_id,
    runId: last.run_id,
    events,
    outcome,
    startedAt: first.ts,
    endedAt: last.ts,
  };
}

/**
 * 将扁平事件归并为运行摘要（新→旧）。
 * 切分：spawn/started 开启新段；done/fail/cancelled/*_failed 结束段。
 */
export function groupEventsIntoRuns(events: RunEvent[]): RunSummary[] {
  const asc = [...events].sort((a, b) => a.ts.localeCompare(b.ts));
  const out: RunSummary[] = [];
  const identified = new Map<string, RunEvent[]>();
  const legacy = new Map<string, RunEvent[]>();
  for (const event of asc) {
    const map = event.run_id ? identified : legacy;
    const key = event.run_id ? JSON.stringify([event.task_id, event.run_id]) : event.task_id;
    const group = map.get(key) ?? [];
    group.push(event); map.set(key, group);
  }
  for (const group of identified.values()) {
    const summary = toSummary(group, isTerminalAction(group.at(-1)?.action ?? ""));
    if (summary) out.push(summary);
  }
  let buf: RunEvent[] = [];

  const flush = (closed: boolean) => {
    const s = toSummary(buf, closed);
    if (s) out.push(s);
    buf = [];
  };

  for (const group of legacy.values()) {
  for (const ev of group) {
    const action = (ev.action ?? "").trim();
    if (isStartAction(action) && buf.length > 0) {
      flush(false);
    }
    buf.push(ev);
    if (isTerminalAction(action)) {
      flush(true);
    }
  }
  if (buf.length) flush(false);
  }

  out.sort((a, b) => b.endedAt.localeCompare(a.endedAt));
  return out;
}
