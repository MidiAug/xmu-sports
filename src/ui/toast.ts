/** 全局应用内 Toast（非系统通知）。见 docs/superpowers/specs/2026-09-22-app-toast-outcome-design.md */

export type ToastKind = "success" | "info" | "warn" | "error";

export type ToastOpts = {
  kind: ToastKind;
  title: string;
  body?: string;
  /** ms；省略则按 kind 默认 */
  durationMs?: number;
  action?: { label: string; run: () => Promise<void> };
};

const DEFAULT_MS: Record<ToastKind, number> = {
  success: 3000,
  info: 3000,
  warn: 5000,
  error: 7000,
};

const MAX_VISIBLE = 2;
const DEDUPE_MS = 2500;

type Entry = {
  id: number;
  el: HTMLElement;
  durationMs: number;
  timer: number;
  paused: boolean;
  acting: boolean;
};

let host: HTMLElement | null = null;
let seq = 0;
const visible: Entry[] = [];
const recentKeys = new Map<string, number>();

function ensureHost(): HTMLElement {
  if (host?.isConnected) return host;
  host = document.createElement("div");
  host.id = "toast_host";
  host.className = "toast-host";
  host.setAttribute("aria-live", "polite");
  document.body.appendChild(host);
  return host;
}

function dedupeKey(opts: ToastOpts): string {
  return `${opts.kind}|${opts.title}|${opts.body ?? ""}`;
}

function clearTimer(entry: Entry) {
  if (entry.timer) {
    window.clearTimeout(entry.timer);
    entry.timer = 0;
  }
}

function scheduleDismiss(entry: Entry, ms: number) {
  clearTimer(entry);
  if (entry.acting) return;
  entry.timer = window.setTimeout(() => dismiss(entry.id), ms);
}

function dismiss(id: number) {
  const i = visible.findIndex((e) => e.id === id);
  if (i < 0) return;
  const [entry] = visible.splice(i, 1);
  clearTimer(entry);
  entry.el.classList.add("toast-leave");
  window.setTimeout(() => entry.el.remove(), 180);
}

export function toast(opts: ToastOpts): void {
  const key = dedupeKey(opts);
  const now = Date.now();
  const last = recentKeys.get(key);
  if (!opts.action && last != null && now - last < DEDUPE_MS) return;
  recentKeys.set(key, now);

  const root = ensureHost();
  while (visible.length >= MAX_VISIBLE) {
    dismiss(visible[0]!.id);
  }

  const id = ++seq;
  const durationMs = opts.durationMs ?? DEFAULT_MS[opts.kind];
  const el = document.createElement("div");
  el.className = `toast toast-${opts.kind}`;
  el.setAttribute("role", opts.kind === "error" || opts.kind === "warn" ? "alert" : "status");
  el.innerHTML = `
    <div class="toast-body">
      <p class="toast-title"></p>
      ${opts.body ? `<p class="toast-text"></p>` : ""}
    </div>
    <button type="button" class="toast-close" aria-label="关闭">×</button>
  `;
  el.querySelector(".toast-title")!.textContent = opts.title;
  const textEl = el.querySelector(".toast-text");
  if (textEl && opts.body) textEl.textContent = opts.body;
  el.querySelector(".toast-close")!.addEventListener("click", () => dismiss(id));

  const entry: Entry = { id, el, durationMs, timer: 0, paused: false, acting: false };
  if (opts.action) {
    const action = document.createElement("button");
    action.type = "button"; action.className = "toast-action"; action.textContent = opts.action.label;
    action.onclick = async () => {
      action.disabled = true; entry.acting = true; clearTimer(entry);
      try { await opts.action!.run(); dismiss(id); }
      catch (error) { entry.acting = false; toastError("操作失败", String(error)); action.disabled = false; }
    };
    el.insertBefore(action, el.querySelector(".toast-close"));
  }
  el.addEventListener("mouseenter", () => {
    entry.paused = true;
    clearTimer(entry);
  });
  el.addEventListener("mouseleave", () => {
    if (el.contains(document.activeElement)) return;
    if (!entry.paused) return;
    entry.paused = false;
    scheduleDismiss(entry, Math.min(entry.durationMs, 2500));
  });
  el.addEventListener("focusin", () => { entry.paused = true; clearTimer(entry); });
  el.addEventListener("focusout", event => {
    if (event.relatedTarget instanceof Node && el.contains(event.relatedTarget)) return;
    if (!el.matches(":hover")) { entry.paused = false; scheduleDismiss(entry, Math.min(entry.durationMs, 2500)); }
  });

  root.appendChild(el);
  visible.push(entry);
  requestAnimationFrame(() => el.classList.add("toast-in"));
  scheduleDismiss(entry, durationMs);
}

export function toastSuccess(title: string, body?: string) {
  toast({ kind: "success", title, body });
}

export function toastInfo(title: string, body?: string) {
  toast({ kind: "info", title, body });
}

export function toastWarn(title: string, body?: string) {
  toast({ kind: "warn", title, body });
}

export function toastError(title: string, body?: string) {
  toast({ kind: "error", title, body });
}
