/** 应用内确认弹窗；样式复用 close-ask。 */

export type ConfirmAskOpts = {
  title: string;
  lead: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** 确认按钮用危险样式（清空/删除类） */
  danger?: boolean;
};

function confirmRoot(): HTMLElement {
  let el = document.getElementById("confirm_ask_root");
  if (!el) {
    el = document.createElement("div");
    el.id = "confirm_ask_root";
    document.body.appendChild(el);
  }
  return el;
}

function hideConfirmAsk() {
  confirmRoot().innerHTML = "";
}

export function askConfirm(opts: ConfirmAskOpts): Promise<boolean> {
  const root = confirmRoot();
  if (root.childElementCount) return Promise.resolve(false);
  const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const confirmLabel = opts.confirmLabel ?? "确认";
  const cancelLabel = opts.cancelLabel ?? "取消";
  const confirmClass = opts.danger ? "btn ghost confirm-ask-danger" : "btn primary";

  return new Promise((resolve) => {
    let finished = false;
    const finish = (ok: boolean) => {
      if (finished) return;
      finished = true;
      window.removeEventListener("keydown", onKey);
      hideConfirmAsk();
      if (previousFocus?.isConnected) previousFocus.focus();
      resolve(ok);
    };

    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Tab") {
        const buttons = Array.from(root.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
        const first = buttons[0], last = buttons.at(-1);
        if (ev.shiftKey && document.activeElement === first) { ev.preventDefault(); last?.focus(); }
        else if (!ev.shiftKey && document.activeElement === last) { ev.preventDefault(); first?.focus(); }
      }
      if (ev.key === "Escape") {
        ev.preventDefault();
        finish(false);
      }
    };

    root.innerHTML = `
      <div class="close-ask-backdrop" role="presentation">
        <div class="close-ask" role="dialog" aria-modal="true" aria-labelledby="confirm_ask_title">
          <h2 id="confirm_ask_title" class="close-ask-title"></h2>
          <p class="close-ask-lead"></p>
          <div class="close-ask-actions">
            <button type="button" class="btn ghost" data-confirm-cancel>${cancelLabel}</button>
            <button type="button" class="${confirmClass}" data-confirm-ok>${confirmLabel}</button>
          </div>
        </div>
      </div>
    `;

    root.querySelector(".close-ask-title")!.textContent = opts.title;
    root.querySelector(".close-ask-lead")!.textContent = opts.lead;

    const backdrop = root.querySelector(".close-ask-backdrop") as HTMLElement;
    const dialog = root.querySelector(".close-ask") as HTMLElement;
    backdrop.addEventListener("click", () => finish(false));
    dialog.addEventListener("click", (e) => e.stopPropagation());
    root.querySelector("[data-confirm-cancel]")!.addEventListener("click", () => finish(false));
    root.querySelector("[data-confirm-ok]")!.addEventListener("click", () => finish(true));
    window.addEventListener("keydown", onKey);
    root.querySelector<HTMLButtonElement>("[data-confirm-cancel]")?.focus();
  });
}
