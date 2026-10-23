import {

  attachConsole,

  debug as pluginDebug,

  error as pluginError,

  info as pluginInfo,

  warn as pluginWarn,

} from "@tauri-apps/plugin-log";



export type LogLevel = "debug" | "info" | "warn" | "error";



type Scalar = string | number | boolean | null | undefined;

export type LogData = Record<string, Scalar | Scalar[]>;



function pad2(n: number): string {

  return String(n).padStart(2, "0");

}



function pad3(n: number): string {

  return String(n).padStart(3, "0");

}



/** 本地时钟 HH:mm:ss.SSS（纯 Vite 回退用） */

export function formatLogTime(d = new Date()): string {

  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}.${pad3(d.getMilliseconds())}`;

}



function formatData(data?: LogData): string {

  if (!data) return "";

  const parts: string[] = [];

  for (const [k, v] of Object.entries(data)) {

    if (v === undefined) continue;

    if (Array.isArray(v)) parts.push(`${k}=[${v.map(String).join(",")}]`);

    else if (v === null) parts.push(`${k}=null`);

    else parts.push(`${k}=${String(v)}`);

  }

  return parts.length ? ` ${parts.join(" ")}` : "";

}



function levelTag(level: LogLevel): string {

  return level.toUpperCase();

}



/** 纯 Vite 回退整行 */

function consoleLine(level: LogLevel, pkg: string, event: string, data?: LogData): string {

  return `[${formatLogTime()}] [${levelTag(level)}] [${pkg}] [${event}]${formatData(data)}`;

}



/** 插件 message：`[pkg] [event] k=v`；Rust format 再加时间与级别 */

function pluginMessage(pkg: string, event: string, data?: LogData): string {

  return `[${pkg}] [${event}]${formatData(data)}`;

}



function consoleFallback(level: LogLevel, line: string) {

  const fn =

    level === "debug"

      ? console.debug

      : level === "info"

        ? console.info

        : level === "warn"

          ? console.warn

          : console.error;

  fn.call(console, line);

}



/** 单通道：只走 plugin；非 Tauri 才 console */

function emit(level: LogLevel, pkg: string, event: string, data?: LogData) {

  const text = pluginMessage(pkg, event, data);

  void (async () => {

    try {

      if (level === "debug") await pluginDebug(text);

      else if (level === "info") await pluginInfo(text);

      else if (level === "warn") await pluginWarn(text);

      else await pluginError(text);

    } catch {

      consoleFallback(level, consoleLine(level, pkg, event, data));

    }

  })();

}



export const log = {

  debug: (pkg: string, event: string, data?: LogData) => emit("debug", pkg, event, data),

  info: (pkg: string, event: string, data?: LogData) => emit("info", pkg, event, data),

  warn: (pkg: string, event: string, data?: LogData) => emit("warn", pkg, event, data),

  error: (pkg: string, event: string, data?: LogData) => emit("error", pkg, event, data),

  /** DEBUG 耗时；event 为动作名 */

  time(pkg: string, event: string, extra?: LogData): () => number {

    const t0 = performance.now();

    return () => {

      const ms = Math.round(performance.now() - t0);

      emit("debug", pkg, event, { ...extra, duration_ms: ms });

      return ms;

    };

  },

};



/** 把 Rust/插件日志映到 DevTools；返回 detach */

export async function attachAppConsole(): Promise<() => void> {

  try {

    return await attachConsole();

  } catch (e) {

    log.warn("log", "attach_console_failed", { err: String(e) });

    return () => {};

  }

}


