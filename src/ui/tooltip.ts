/** Shared hints for icon controls and brief contextual information. */
export function mountTooltips() {
  const hint = document.createElement("div");
  hint.id = "shell_tooltip";
  hint.className = "shell-tooltip";
  hint.setAttribute("role", "tooltip");
  hint.hidden = true;
  document.body.append(hint);
  let active: HTMLElement | null = null;
  let timer = 0;
  function hide() {
    window.clearTimeout(timer);
    if (active) {
      const ids = (active.getAttribute("aria-describedby") ?? "").split(" ").filter(id => id && id !== hint.id);
      if (ids.length) active.setAttribute("aria-describedby", ids.join(" "));
      else active.removeAttribute("aria-describedby");
    }
    active = null;
    hint.hidden = true;
  }
  function show(target: EventTarget | null) {
    const control = target instanceof Element ? target.closest<HTMLElement>("[data-tooltip]") : null;
    if (control === active) return;
    hide();
    if (!control) return;
    active = control;
    timer = window.setTimeout(() => {
      if (!control.isConnected) { hide(); return; }
      hint.textContent = control.dataset.tooltip || control.getAttribute("aria-label");
      if (!hint.textContent) return;
      hint.hidden = false;
      const box = control.getBoundingClientRect(), tip = hint.getBoundingClientRect();
      hint.style.left = `${Math.max(8, Math.min(innerWidth - tip.width - 8, box.x + (box.width - tip.width) / 2))}px`;
      hint.style.top = `${box.bottom + tip.height + 16 < innerHeight ? box.bottom + 8 : box.top - tip.height - 8}px`;
      control.setAttribute("aria-describedby", [control.getAttribute("aria-describedby"), hint.id].filter(Boolean).join(" "));
    }, 350);
  }
  document.addEventListener("pointerover", event => show(event.target));
  document.addEventListener("pointerout", event => { if (active && !active.contains(event.relatedTarget as Node | null)) hide(); });
  document.addEventListener("focusin", event => show(event.target));
  document.addEventListener("focusout", hide);
  document.addEventListener("keydown", event => { if (event.key === "Escape") hide(); });
  document.addEventListener("pointerdown", hide);
  document.addEventListener("scroll", hide, true);
  window.addEventListener("resize", hide);
}
