import type { LiveStatus } from "../types";
import { setLiveStatusCache } from "../state";
import { setRunIndicator } from "./run-indicator";
import {
  outcomeFromEvent,
  outcomeKey,
  toastPayloadFromOutcome,
} from "./run-outcome";
import { toast, toastInfo } from "../ui/toast";

/** 跑态边沿 → Toast；与 Tab 无关。见 2026-09-22-app-toast-outcome-design.md */

let seeded = false;
let prevRunning = false;
let lastToastedOutcomeKey = "";

export function seedRunFeedback(s: LiveStatus) {
  prevRunning = Boolean(s.running);
  const o = outcomeFromEvent(s.last_event, s.recent_events);
  lastToastedOutcomeKey = o ? outcomeKey(o) : "";
  seeded = true;
  setLiveStatusCache(s);
  setRunIndicator(Boolean(s.armed || s.running));
}

/**
 * 消化一次 LiveStatus：更新缓存/绿点，并在边沿弹出 toast。
 * 首次调用只 seed，不 toast（避免启动时刷历史失败）。
 */
export function ingestLiveStatus(s: LiveStatus): void {
  setLiveStatusCache(s);
  setRunIndicator(Boolean(s.armed || s.running));

  const running = Boolean(s.running);

  if (!seeded) {
    seedRunFeedback(s);
    return;
  }

  if (running && !prevRunning) {
    toastInfo("已开始执行", s.operation === "recheck" ? "正在只读核对原账号与实际提交场次，不提交预约" : s.operation === "auth" ? "正在检测与恢复体育登录，不提交预约" : s.task?.executor === "http" ? "正在检测登录并查询实时场次" : "HTTP 请求执行中");
  }

  if (!running && prevRunning) {
    const o = outcomeFromEvent(s.last_event, s.recent_events);
    if (o) {
      const key = outcomeKey(o);
      if (key !== lastToastedOutcomeKey) {
        lastToastedOutcomeKey = key;
        toast(toastPayloadFromOutcome(o));
      }
    } else {
      toastInfo("已结束", "预约流程已停止，请查看执行记录");
    }
  }

  prevRunning = running;
}
