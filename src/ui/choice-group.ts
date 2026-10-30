import type { SelectOption } from "../types";

/** Direct choices share the same option values and availability badges as menus. */
export function mountChoiceGroup(
  host: HTMLElement, options: SelectOption[], selected: string,
  onChange: (value: string) => void,
  config: { label: string; kind: "slot" | "sport"; icon?: (value: string) => string },
) {
  let value = selected, choices = options, disabled = false;
  const escape = (text: string) => text.replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]!));
  host.className = `choice-group choice-group--${config.kind}`;
  host.setAttribute("role", "group");
  host.setAttribute("aria-label", config.label);
  function revealSelection() {
    const active = host.querySelector<HTMLElement>(".choice-option.on");
    if (active && config.kind === "slot") host.scrollTop = active.offsetTop - (host.clientHeight - active.offsetHeight) / 2;
  }
  function render() {
    host.innerHTML = choices.map(option => `<button type="button" class="choice-option${option.value === value ? " on" : ""}" data-value="${escape(option.value)}" aria-pressed="${option.value === value}" ${disabled || option.disabled ? "disabled" : ""}>
      <span class="choice-main">${config.icon ? `<span class="sport-icon" data-sport="${escape(option.value)}">${config.icon(option.value)}</span>` : ""}<span class="choice-label">${escape(option.label)}</span></span>${option.badge ? `<span class="shell-select-badge ${option.disabled ? "badge-locked" : option.badge === "已满" ? "badge-full" : "badge-partial"}">${escape(option.badge)}</span>` : ""}</button>`).join("");
    host.classList.toggle("has-disabled-value", !!choices.find(option => option.value === value)?.disabled);
    revealSelection();
  }
  host.addEventListener("click", event => {
    const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-value]");
    if (!button || button.disabled) return;
    value = button.dataset.value!;
    host.querySelectorAll<HTMLButtonElement>("[data-value]").forEach(item => {
      const active = item.dataset.value === value;
      item.classList.toggle("on", active); item.setAttribute("aria-pressed", String(active));
    });
    onChange(value);
  });
  host.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    const buttons = [...host.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (index < 0 || !buttons.length) return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1 : (index + (event.key === "ArrowLeft" ? -1 : 1) + buttons.length) % buttons.length;
    buttons[next].focus();
  });
  render();
  const resize = config.kind === "slot" ? new ResizeObserver(revealSelection) : undefined;
  resize?.observe(host);
  return {
    dispose: () => resize?.disconnect(),
    getValue: () => value,
    setOptions: (next: SelectOption[], selection?: string) => {
      const nextValue = selection ?? (next.some(option => option.value === value) ? value : next.find(option => !option.disabled)?.value ?? "");
      if (nextValue === value && JSON.stringify(next) === JSON.stringify(choices)) return;
      choices = next; value = nextValue; render();
    },
    setDisabled: (next: boolean) => {
      disabled = next;
      host.querySelectorAll<HTMLButtonElement>("[data-value]").forEach(button => {
        button.disabled = disabled || !!choices.find(option => option.value === button.dataset.value)?.disabled;
      });
    },
  };
}
