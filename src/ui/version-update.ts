import { watchAppUpdate } from "../lib/app-update";

/** Updates stay discoverable without adding a version control to the title bar. */
export function mountSettingsUpdateIndicator(button: HTMLButtonElement) {
  const dot = document.createElement("span");
  dot.className = "settings-update-dot";
  dot.setAttribute("aria-hidden", "true");
  button.append(dot);
  return watchAppUpdate(state => {
    const available=state.release?.status==="available";
    const ready=state.phase==="downloaded" || state.phase==="installing" || state.phase==="error" && state.failedAt==="install";
    dot.hidden=!available;
    button.classList.toggle("has-update",available);
    if (available) button.setAttribute("aria-description", ready ? "更新已下载，可在关于中安装" : "有更新，可在关于中查看");
    else button.removeAttribute("aria-description");
  });
}
