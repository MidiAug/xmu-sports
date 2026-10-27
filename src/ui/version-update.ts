import { watchAppUpdate } from "../lib/app-update";

/** The shell keeps the detected release visible while the user visits other pages. */
export function mountVersionUpdate(foot: Element, openAbout: () => void) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "version-update";
  button.hidden = true;
  button.innerHTML = '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 16V4m-4 4 4-4 4 4M5 14v5a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  button.onclick = openAbout;
  foot.append(button);
  return watchAppUpdate(state => {
    button.hidden = state.release?.status !== "available";
    button.title = state.release ? `有新版本 ${state.release.version}，查看应用更新` : "查看应用更新";
    button.setAttribute("aria-label", button.title);
  });
}
