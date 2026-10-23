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
  else await invoke("quit_app");
}

export function showCloseAsk(busy = false) {
  const root = closeAskRoot();
  let remember = true;
  let busyFlag = false;

  if (!busy) {
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
    const title = busy ? "挂机进行中" : "关闭窗口";
    const lead = busy
      ? "当前有挂机或执行任务。最小化到托盘可继续等待开抢；退出将取消挂机并结束进程。"
      : "挂机开抢时建议最小化到托盘，避免误关进程。";
    const quitLabel = busy ? "退出并取消挂机" : "退出应用";
    const rememberBlock = busy
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
            }>最小化到托盘</button>
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
        await applyCloseChoice(true, remember, false, !busy);
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
        await applyCloseChoice(false, remember, busy, !busy);
      } catch (e) {
        busyFlag = false;
        paint();
        console.error(e);
      }
    });
  }
  paint();
}
