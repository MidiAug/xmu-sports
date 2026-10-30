import { invoke } from "@tauri-apps/api/core";
import type { ShellPrefs } from "../types";
import { setRunIndicator } from "../lib/run-indicator";
import { updatePrefs } from "../lib/prefs-store";

function closeAskRoot(): HTMLElement {
  let el = document.getElementById("close_ask_root");
  if (!el) {
    el = document.createElement("div");
    el.id = "close_ask_root";
    document.body.appendChild(el);
  }
  return el;
}

export function hideCloseAsk() {
  closeAskRoot().innerHTML = "";
  window.removeEventListener("keydown", onCloseAskKeydown);
}

function onCloseAskKeydown(ev: KeyboardEvent) {
  if (ev.key === "Tab") {
    const controls = [...closeAskRoot().querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")];
    const first = controls[0], last = controls[controls.length - 1];
    if (first && (ev.shiftKey ? document.activeElement === first : document.activeElement === last)) {
      ev.preventDefault();
      (ev.shiftKey ? last : first).focus();
    }
  }
  if (ev.key === "Escape") {
    ev.preventDefault();
    hideCloseAsk();
  }
}

async function applyCloseChoice(
  toTray: boolean,
  remember: boolean,
  disarm: boolean,
  savePrefs: boolean,
  confirmed = false,
) {
  if (savePrefs) {
    try {
      await updatePrefs({
        close_to_tray: toTray,
        close_pref_set: remember,
        close_remember_default_used: true,
      });
    } catch (error) {
      console.warn("关闭偏好未保存", error);
    }
  }
  if (disarm) {
    try {
      await invoke("disarm_hang");
      setRunIndicator(false);
    } catch (e) {
      console.error(e);
    }
  }
  hideCloseAsk();
  if (toTray) await invoke("hide_to_tray");
  else await invoke(confirmed ? "force_quit_app" : "quit_app");
}

export function showCloseAsk(busy = false, plans = false) {
  const guarded = busy || plans;
  const root = closeAskRoot();
  let remember = true;
  let busyFlag = false;

  if (!guarded) {
    void invoke<ShellPrefs>("get_shell_prefs")
      .then((p) => {
        const defaultUsed = Boolean(p.close_remember_default_used);
        remember = !defaultUsed;
        const box = root.querySelector("#close_ask_remember") as HTMLInputElement | null;
        if (box) box.checked = remember;
      })
      .catch(() => {});
  }

  window.removeEventListener("keydown", onCloseAskKeydown);
  window.addEventListener("keydown", onCloseAskKeydown);

  function paint() {
    const title = busy ? "预约任务进行中" : plans ? "自动预约已开启" : "关闭窗口";
    const lead = guarded
      ? `${busy ? "当前任务" : "长期计划"}需要应用保持运行。留在托盘可继续预约；退出后${busy ? "停止当前任务，并" : ""}停止后台自动预约。计划会保留，下次启动后继续生效，已提交的预约不会取消。`
      : "挂机开抢时建议最小化到托盘，避免误关进程。";
    const quitLabel = guarded ? "停止并退出" : "退出应用";
    const rememberBlock = guarded
      ? ""
      : `<label class="close-ask-remember">
            <input type="checkbox" id="close_ask_remember" ${remember ? "checked" : ""} ${
              busyFlag ? "disabled" : ""
            } />
            <span>记住我的选择</span>
          </label>`;
    root.innerHTML = `
      <div class="close-ask-backdrop" role="presentation">
        <div class="close-ask" role="dialog" aria-modal="true" aria-labelledby="close_ask_title">
          <h2 id="close_ask_title" class="close-ask-title">${title}</h2>
          <p class="close-ask-lead">${lead}</p>
          ${rememberBlock}
          <div class="close-ask-actions">
            <button type="button" class="btn primary" id="close_ask_tray" ${
              busyFlag ? "disabled" : ""
            }>${guarded ? "留在托盘" : "最小化到托盘"}</button>
            <button type="button" class="btn ghost" id="close_ask_quit" ${
              busyFlag ? "disabled" : ""
            }>${quitLabel}</button>
          </div>
        </div>
      </div>
    `;
    const backdrop = root.querySelector(".close-ask-backdrop") as HTMLElement;
    const dialog = root.querySelector(".close-ask") as HTMLElement;
    backdrop.addEventListener("click", () => {
      if (!busyFlag) hideCloseAsk();
    });
    dialog.addEventListener("click", (e) => e.stopPropagation());
    root.querySelector("#close_ask_remember")?.addEventListener("change", (ev) => {
      remember = (ev.target as HTMLInputElement).checked;
    });
    root.querySelector("#close_ask_tray")?.addEventListener("click", async () => {
      if (busyFlag) return;
      busyFlag = true;
      paint();
      try {
        await applyCloseChoice(true, remember, false, !guarded);
      } catch (e) {
        busyFlag = false;
        paint();
        console.error(e);
      }
    });
    root.querySelector("#close_ask_quit")?.addEventListener("click", async () => {
      if (busyFlag) return;
      busyFlag = true;
      paint();
      try {
        await applyCloseChoice(false, remember, busy, !guarded, guarded);
      } catch (e) {
        busyFlag = false;
        paint();
        console.error(e);
      }
    });
  }
  paint();
  root.querySelector<HTMLButtonElement>("#close_ask_tray")?.focus();
}
