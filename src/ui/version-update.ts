import { watchAppUpdate } from "../lib/app-update";

/** The shell keeps the detected release visible while the user visits other pages. */
export function mountVersionUpdate(foot: Element, openAbout: () => void) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "version-update";
  const version=foot.querySelector(".nav-version");
  if(version)button.append(version);
  const status=document.createElement("span");
  status.className="version-update-status";
  status.innerHTML='<span aria-hidden="true">↑</span><span class="version-update-label"></span>';
  button.append(status);
  button.onclick = openAbout;
  foot.append(button);
  return watchAppUpdate(state => {
    const available=state.release?.status==="available";
    const ready=state.phase==="downloaded" || state.phase==="installing" || state.phase==="error" && state.failedAt==="install";
    status.hidden=!available;
    status.querySelector(".version-update-label")!.textContent=ready ? "可安装" : "有更新";
    button.classList.toggle("has-update",available);
    if(available)button.title=`新版本 ${state.release!.version}`;
    else button.removeAttribute("title");
    button.setAttribute("aria-label",available
      ? ready ? `更新 ${state.release!.version} 已下载，查看安装` : `有新版本 ${state.release!.version}，查看应用更新`
      : "关于与应用更新");
  });
}
