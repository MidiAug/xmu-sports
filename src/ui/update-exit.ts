import { invoke } from "@tauri-apps/api/core";
import { watchAppUpdate, installAppUpdate } from "../lib/app-update";

/** Exit only: background checks and hiding to tray never open this dialog. */
export function showUpdateExit(openUpdates:()=>void, plansEnabled=false) {
  if(document.getElementById("update_exit_root")) return;
  const root=document.createElement("div");root.id="update_exit_root";
  root.innerHTML='<div class="close-ask-backdrop"><section class="close-ask update-exit" role="dialog" aria-modal="true" aria-labelledby="update_exit_title"><h2 class="close-ask-title" id="update_exit_title">有可用更新</h2><p class="close-ask-lead" role="status"></p><div class="close-ask-actions"><button class="btn ghost" data-update-exit>直接退出</button><button class="btn primary" data-update-open>查看更新</button></div><button class="update-exit-cancel" aria-label="取消退出">×</button></section></div>';
  document.body.append(root);
  const previous=document.activeElement;
  const action=root.querySelector<HTMLButtonElement>("[data-update-open]")!;
  let ready=false;
  const stop=watchAppUpdate(state=>{
    ready=state.phase==="downloaded" || state.phase==="error" && state.failedAt==="install";
    action.disabled=state.phase==="installing";
    action.textContent=ready ? "安装并退出" : state.phase==="downloading" ? "继续下载" : "查看更新";
    root.querySelector(".close-ask-lead")!.textContent=state.phase==="error" && state.failedAt==="install"
      ? state.error ?? "安装未能启动，请稍后重试"
      : ready ? "更新已下载并校验，可以安装后退出。"
      : state.phase==="downloading" ? "更新正在下载，直接退出会中断本次下载。" : "发现新版本，现在查看更新，或下次再安装。";
    if(plansEnabled) root.querySelector(".close-ask-lead")!.textContent+=" 退出后自动预约将暂停。";
  });
  const close=()=>{stop();root.remove();window.removeEventListener("keydown",keydown);if(previous instanceof HTMLElement && previous.isConnected)previous.focus();};
  const keydown=(event:KeyboardEvent)=>{
    if(event.key==="Escape"){event.preventDefault();close();}
    if(event.key==="Tab"){
      const buttons=Array.from(root.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
      if(event.shiftKey && document.activeElement===buttons[0]){event.preventDefault();buttons.at(-1)?.focus();}
      else if(!event.shiftKey && document.activeElement===buttons.at(-1)){event.preventDefault();buttons[0]?.focus();}
    }
  };
  window.addEventListener("keydown",keydown);
  root.querySelector(".update-exit-cancel")!.addEventListener("click",close);
  root.querySelector(".close-ask-backdrop")!.addEventListener("click",event=>{if(event.target===event.currentTarget)close();});
  root.querySelector<HTMLButtonElement>("[data-update-exit]")!.onclick=()=>{void invoke("force_quit_app").catch(()=>{root.querySelector(".close-ask-lead")!.textContent="退出未完成，请重试";});};
  action.onclick=()=>{if(ready)void installAppUpdate();else{close();openUpdates();}};
  action.focus();
}
