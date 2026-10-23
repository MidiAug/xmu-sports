export function setMsg(el: Element | null, text: string, kind: "" | "ok" | "err" = "") {
  if (!el) return;
  el.className = `msg${kind ? ` ${kind}` : ""}`;
  el.textContent = text;
}
