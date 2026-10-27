import { version } from "../../../package.json";
import type { ReleaseInfo,UpdateView } from "../../lib/app-update";
import { renderAbout, type AboutUpdateSource } from "../about";
import { mountPage,disposePage } from "../../lib/page-lifecycle";
import { mountShellSelect } from "../../ui/select";
import { mountVersionUpdate } from "../../ui/version-update";

const scenarios=[
  {value:"idle",label:"初始状态"},{value:"checking",label:"检查中"},
  {value:"current",label:"已是最新"},{value:"unpublished",label:"尚未发布"},
  {value:"available",label:"有更新"},{value:"downloading",label:"下载中"},
  {value:"downloaded",label:"可安装"},{value:"installing",label:"安装中"},
  {value:"check-error",label:"检查失败"},{value:"download-error",label:"下载失败"},
  {value:"install-error",label:"安装受阻"},{value:"paused",label:"预约期间暂停"},
];
const release:ReleaseInfo={status:"available",version:"0.2.0",size:42*1024*1024,notes:[
  '## 更新内容','','- **预约体验**：跨零点保留实际预约日期。','- 改善缓存、线路选择与更新提示。',
  '','### 使用说明','','这是开发预览的模拟说明，用于检查长文本、段落和列表的显示。',
  '','| 状态 | 说明 |','| --- | --- |','| 下载完成 | 校验后等待确认安装 |','',
  '[查看项目](https://github.com/MidiAug/xmu-sports)',
].join('\n')};

/** Only imported behind import.meta.env.DEV; never patches the real update singleton. */
export async function renderAboutPreview(panel:Element,back:()=>void){
  if(!import.meta.env.DEV)return;
  const scope=mountPage(panel);
  const previousTheme=document.documentElement.dataset.shellTheme;
  const previousScheme=document.documentElement.style.colorScheme;
  scope.onDispose(()=>{
    if(previousTheme===undefined)delete document.documentElement.dataset.shellTheme;
    else document.documentElement.dataset.shellTheme=previousTheme;
    document.documentElement.style.colorScheme=previousScheme;
  });
  panel.innerHTML='<div class="page-stack dev-about-preview"><header class="page-heading"><div><h2>关于页状态预览</h2><p>模拟更新流程，不会真实检查、下载或安装</p></div></header><section class="preference-card dev-preview-controls"><div class="dev-preview-control"><label id="preview_scenario_label">预览状态</label><div id="preview_scenario" aria-labelledby="preview_scenario_label"></div></div><div class="dev-preview-control"><span>外观</span><div class="dev-preview-buttons"><button class="btn ghost" data-preview-theme="light">浅色</button><button class="btn ghost" data-preview-theme="dark">深色</button></div></div><div class="dev-preview-control"><span>内容宽度</span><div class="dev-preview-buttons"><button class="btn ghost" data-preview-width="auto" aria-pressed="true">跟随窗口</button><button class="btn ghost" data-preview-width="360" aria-pressed="false">窄内容</button></div></div><div class="dev-preview-control"><span>版本入口</span><div id="preview_version" class="nav-foot"><span class="nav-version"></span></div></div><label class="dev-preview-progress" hidden>下载进度 <input type="range" min="0" max="100" value="42" id="preview_progress"><output>42%</output></label><p class="msg" id="preview_notice" role="status"></p></section><div id="preview_canvas"></div></div>';
  const canvas=panel.querySelector<HTMLElement>("#preview_canvas")!;
  scope.onDispose(()=>disposePage(canvas));
  const notice=panel.querySelector<HTMLElement>("#preview_notice")!;
  const progress=panel.querySelector<HTMLInputElement>("#preview_progress")!;
  const watchers=new Set<(value:UpdateView)=>void>();
  let state:UpdateView={phase:"ready",progress:0,release};
  let route="official";
  const publish=(value:UpdateView)=>{
    state=value;progress.disabled=state.phase!=="downloading";
    panel.querySelector<HTMLElement>(".dev-preview-progress")!.hidden=progress.disabled;
    progress.value=String(state.progress);
    panel.querySelector("output")!.textContent=`${state.progress}%`;
    for(const listener of watchers)listener(state);
  };
  const source:AboutUpdateSource={
    watch:listener=>{watchers.add(listener);listener(state);return()=>{watchers.delete(listener);};},
    route:()=>route,setRoute:value=>{route=value;return true;},
    perform:()=>{
      if(state.phase==="downloaded" || state.failedAt==="install")publish({phase:"installing",progress:100,release});
      else if(state.release?.status==="available" && state.failedAt!=="check")publish({phase:"downloading",progress:42,release});
      else publish({phase:"ready",progress:0,release});
      notice.textContent="按钮仅切换模拟状态，未调用更新服务";
    },
  };
  mountShellSelect(panel.querySelector<HTMLElement>("#preview_scenario")!,scenarios,"available",value=>{
    notice.textContent="";
    if(value==="current" || value==="unpublished")publish({phase:"ready",progress:0,release:{...release,status:value,version:value==="current" ? version : "",notes:"",size:0}});
    else if(value==="paused")publish({phase:"ready",progress:0,release,paused:true});
    else if(value.endsWith("-error")){
      const failedAt=value==="check-error" ? "check" : value==="download-error" ? "download" : "install";
      publish({phase:"error",failedAt,progress:0,release:failedAt==="check" ? undefined : release,error:failedAt==="check" ? "GitHub 请求受限，请稍后重试或查看发布页" : failedAt==="download" ? "下载连接中断，请更换线路后重试" : "请先结束预约、取消定时任务或完成登录，再安装更新"});
    } else if(value==="available")publish({phase:"ready",progress:0,release});
    else publish({phase:value as UpdateView["phase"],progress:value==="downloading" ? 42 : value==="downloaded" || value==="installing" ? 100 : 0,release:value==="idle" || value==="checking" ? undefined : release});
  },{matchTrigger:true});
  const foot=panel.querySelector<HTMLElement>("#preview_version")!;
  foot.querySelector(".nav-version")!.textContent=`v${version}`;
  scope.onDispose(mountVersionUpdate(foot,()=>canvas.scrollIntoView({block:"start",behavior:"smooth"}),source.watch));
  progress.oninput=()=>publish({...state,progress:Number(progress.value)});
  panel.querySelectorAll<HTMLButtonElement>("[data-preview-theme]").forEach(button=>{
    button.setAttribute("aria-pressed",String(button.dataset.previewTheme===previousTheme));
    button.onclick=()=>{
      document.documentElement.dataset.shellTheme=button.dataset.previewTheme;
      document.documentElement.style.colorScheme=button.dataset.previewTheme!;
      panel.querySelectorAll<HTMLButtonElement>("[data-preview-theme]").forEach(item=>item.setAttribute("aria-pressed",String(item===button)));
    };
  });
  panel.querySelectorAll<HTMLButtonElement>("[data-preview-width]").forEach(button=>{
    button.onclick=()=>{
      canvas.style.maxWidth=button.dataset.previewWidth==="auto" ? "" : "360px";
      panel.querySelectorAll<HTMLButtonElement>("[data-preview-width]").forEach(item=>item.setAttribute("aria-pressed",String(item===button)));
    };
  });
  await renderAbout(canvas,back,{updates:source,version,openLink:async()=>{notice.textContent="预览模式不会打开外部链接";}});
}
