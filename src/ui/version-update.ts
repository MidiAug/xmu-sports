import { watchAppUpdate } from "../lib/app-update";

/** The shell keeps the detected release visible while the user visits other pages. */
export function mountVersionUpdate(foot: Element, openAbout: () => void) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "version-update";
  button.hidden = true;
  button.innerHTML = '<svg width="14" height="14" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m5.5 9 4.5-4.5L14.5 9M5.5 14.5 10 10l4.5 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  button.onclick = openAbout;
  foot.append(button);
  return watchAppUpdate(state => {
    button.hidden = state.release?.status !== "available";
    button.title = state.release ? `有新版本 ${state.release.version}，查看应用更新` : "查看应用更新";
    button.setAttribute("aria-label", button.title);
  });
}
