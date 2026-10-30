import type { SelectOption } from "../types";
import { SVG } from "./icons";

let activeSelect: { close: () => void } | undefined;
export function closeShellSelects() { activeSelect?.close(); }

export function mountShellSelect(
  host: HTMLElement,
  options: SelectOption[],
  selected: string,
  onChange: (value: string) => void,
  opts?: { wide?: boolean; columns?: boolean; center?: boolean; compact?: boolean; matchTrigger?:boolean },
): { getValue: () => string; setOptions: (opts: SelectOption[], sel?: string) => void; setDisabled: (disabled: boolean) => void } {
  let value = selected;
  let selectOpts = options;
  const escape = (text: string) => text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");
  const badgeClass = (badge:string) => ["已锁","全部锁定"].includes(badge) ? "badge-locked" : badge==="已满" ? "badge-full" : badge==="部分锁定" ? "badge-partial" : "";
  const wide = Boolean(opts?.wide);
  const forceColumns = Boolean(opts?.columns);
  const center = Boolean(opts?.center);
  const compact = Boolean(opts?.compact);
  const menuId = `select-${crypto.randomUUID()}`;
  const label = host.closest(".settings-cell")?.querySelector(".settings-cell-title")?.textContent ?? "选择选项";
  let cleanup: (() => void) | undefined;
  const close = (element: HTMLElement, focus = false) => {
    cleanup?.(); cleanup = undefined;
    element.classList.remove("open");
    const menu = element.querySelector<HTMLElement>(".shell-select-menu");
    if (typeof menu?.hidePopover === "function" && menu.matches(":popover-open")) menu.hidePopover();
    const button = element.querySelector<HTMLButtonElement>(".shell-select-trigger");
    button?.setAttribute("aria-expanded", "false");
    if (focus) button?.focus();
  };

  function useColumns(): boolean {
    return forceColumns || selectOpts.length >= 6;
  }

  function renderMenu() {
    const menu = host.querySelector(".shell-select-menu") as HTMLElement;
    const valueEl = host.querySelector(".shell-select-value") as HTMLElement;
    const cur = selectOpts.find((o) => o.value === value);
    valueEl.innerHTML = `<span class="shell-select-main">${escape(cur?.label ?? value)}</span>${cur?.badge ? `<span class="shell-select-badge ${badgeClass(cur.badge)}">${escape(cur.badge)}</span>` : ""}`;
    host.classList.toggle("has-option-details", selectOpts.some(o=>o.badge));
    host.classList.toggle("has-disabled-value", !!cur?.disabled);
    menu.classList.toggle("shell-select-menu--cols", useColumns());
    menu.innerHTML = selectOpts
      .map((o) => {
        const on = o.value === value ? "on" : "";
        const checkClass = o.value === value ? "shell-select-check" : "shell-select-check spacer";
        return `<li>
          <button type="button" role="option" aria-selected="${o.value === value}" aria-disabled="${!!o.disabled}" ${o.disabled ? "disabled" : ""} tabindex="-1" class="shell-select-option ${on}" data-value="${escape(o.value)}">
            <span class="shell-select-option-label"><span class="shell-select-option-main">${escape(o.label)}${o.badge ? `<span class="shell-select-badge ${badgeClass(o.badge)}">${escape(o.badge)}</span>` : ""}</span></span>
            <span class="${checkClass}">${SVG.check}</span>
          </button>
        </li>`;
      })
      .join("");
    menu.querySelectorAll("[data-value]").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        if ((btn as HTMLButtonElement).disabled) return;
        value = (btn as HTMLButtonElement).dataset.value!;
        close(host, true);
        renderMenu();
        onChange(value);
      });
    });
  }

  host.className = `shell-select${wide ? " shell-select--wide" : ""}${
    center ? " shell-select--center" : ""
  }`;
  host.innerHTML = `
    <button type="button" class="shell-select-trigger" aria-haspopup="listbox" aria-expanded="false" aria-controls="${menuId}">
      <span class="shell-select-value"></span>
      <span class="shell-select-chevron">${SVG.chevron}</span>
    </button>
    <ul class="shell-select-menu" id="${menuId}" role="listbox"></ul>
  `;
  const trigger = host.querySelector(".shell-select-trigger") as HTMLButtonElement;
  const menu = host.querySelector<HTMLElement>(".shell-select-menu")!;
  const topLayer = typeof menu.showPopover === "function";
  if (topLayer) menu.setAttribute("popover", "manual");
  trigger.setAttribute("aria-label", label);
  menu.setAttribute("aria-label", label);

  function positionMenu() {
    if (!host.isConnected) { close(host); return; }
    const anchor = trigger.getBoundingClientRect();
    const container = host.closest(".content")?.getBoundingClientRect();
    const left = Math.max(8, (container?.left ?? 0) + 8);
    const right = Math.min(window.innerWidth - 8, (container?.right ?? window.innerWidth) - 8);
    const top = Math.max(8, (container?.top ?? 0) + 8);
    const bottom = Math.min(window.innerHeight - 8, (container?.bottom ?? window.innerHeight) - 8);
    if (anchor.bottom < top || anchor.top > bottom || right <= left) { close(host); return; }
    const detailed=selectOpts.some(o=>o.badge);
    const wantedWidth = opts?.matchTrigger ? anchor.width : compact ? Math.min(320, Math.max(280, anchor.width))
      : Math.max(anchor.width, detailed ? 420 : useColumns() ? selectOpts.length >= 9 ? 400 : 300 : anchor.width);
    const width = Math.min(wantedWidth, right - left);
    const columns = useColumns() ? Math.max(1, Math.min(compact || detailed ? 2 : selectOpts.length >= 9 ? 3 : 2, Math.floor((width - 12) / (compact ? 130 : detailed ? 190 : 124)))) : 1;
    menu.style.width = `${width}px`;
    menu.style.setProperty("--select-columns", String(columns));
    menu.style.maxHeight = "none";
    const wantedHeight = Math.min(360, menu.scrollHeight + 2);
    const below = Math.max(0, bottom - anchor.bottom - 8);
    const above = Math.max(0, anchor.top - top - 8);
    const openAbove = below < wantedHeight && above > below;
    const height = Math.min(wantedHeight, openAbove ? above : below);
    menu.style.maxHeight = `${height}px`;
    menu.style.left = `${Math.max(left, Math.min(anchor.left, right - width))}px`;
    menu.style.top = `${openAbove ? anchor.top - 8 - height : anchor.bottom + 8}px`;
  }

  function openMenu() {
    closeShellSelects();
    const entry = { close: () => close(host) };
    activeSelect = entry;
    const controller = new AbortController();
    const observer = new ResizeObserver(positionMenu);
    cleanup = () => { controller.abort(); observer.disconnect(); if (activeSelect === entry) activeSelect = undefined; };
    host.classList.add("open");
    trigger.setAttribute("aria-expanded", "true");
    if (topLayer) menu.showPopover();
    positionMenu();
    if (!host.classList.contains("open")) return;
    observer.observe(host);
    window.addEventListener("resize", positionMenu, { signal: controller.signal });
    window.addEventListener("blur", () => close(host), { signal: controller.signal });
    document.addEventListener("scroll", (event) => {
      if (!(event.target instanceof Node) || !menu.contains(event.target)) close(host);
    }, { capture: true, signal: controller.signal });
  }
  trigger.addEventListener("click", (e) => {
    e.stopPropagation();
    const willOpen = !host.classList.contains("open");
    if (willOpen) openMenu(); else close(host);
  });
  host.addEventListener("keydown", (event) => {
    if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(host, true); return; }
    if (event.key === "Tab") { close(host, menu.contains(document.activeElement)); return; }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    if (!host.classList.contains("open")) {
      openMenu();
      const selected = host.querySelector<HTMLButtonElement>('.shell-select-option[aria-selected="true"]:not(:disabled)');
      (selected ?? host.querySelector<HTMLButtonElement>(".shell-select-option:not(:disabled)"))?.focus(); return;
    }
    const choices = Array.from(host.querySelectorAll<HTMLButtonElement>(".shell-select-option:not(:disabled)"));
    let index = choices.indexOf(document.activeElement as HTMLButtonElement);
    index = event.key === "Home" ? 0 : event.key === "End" ? choices.length - 1 : Math.max(0, Math.min(choices.length - 1, index + (event.key === "ArrowUp" ? -1 : 1)));
    choices[index]?.focus();
  });
  renderMenu();

  return {
    getValue: () => value,
    setDisabled: (disabled) => {
      if (disabled) close(host);
      trigger.disabled = disabled;
    },
    setOptions: (next, sel) => {
      if (host.classList.contains("open")) close(host);
      selectOpts = next;
      if (sel !== undefined) value = sel;
      else if (!selectOpts.some((o) => o.value === value)) value = selectOpts.find(o => !o.disabled)?.value ?? "";
      renderMenu();
    },
  };
}
