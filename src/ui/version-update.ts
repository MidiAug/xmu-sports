import { watchAppUpdate } from "../lib/app-update";

/** The shell keeps the detected release visible while the user visits other pages. */
export function mountVersionUpdate(foot: Element, openAbout: () => void) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "version-update";
  button.hidden = true;
  button.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 16V4m-5 5 5-5 5 5M5 16v4h14v-4" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  button.onclick = openAbout;
  foot.append(button);
  return watchAppUpdate(state => {
    button.hidden = state.release?.status !== "available";
    button.title = state.release ? `有新版本 ${state.release.version}，查看应用更新` : "查看应用更新";
    button.setAttribute("aria-label", button.title);
  });
}
