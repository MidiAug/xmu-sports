import { getVersion } from "@tauri-apps/api/app";
import { open } from "@tauri-apps/plugin-shell";
import { mountPage } from "../lib/page-lifecycle";
import { watchAppUpdate, performAppUpdate } from "../lib/app-update";
import { renderReleaseNotes } from "../lib/release-notes";

const REPOSITORY = "https://github.com/MidiAug/xmu-sports";
const appIcon = new URL("../../assets/app-icon/source.svg", import.meta.url).href;
const updateHint = import.meta.env.DEV ? "开发版仅手动检查更新" : "启动后自动检查，成功后间隔24小时；失败后1小时重试";
export async function renderAbout(panel: Element, back: () => void) {
  const scope = mountPage(panel);
  let version = "版本暂不可用";
  try { version = `版本 ${await getVersion()}`; } catch { /* Browser preview. */ }
  if (!scope.current()) return;
  panel.closest<HTMLElement>(".content")?.scrollTo({ top: 0 });
  panel.innerHTML = `<div class="page-stack settings-page about-page">
    <div class="about-navigation"><button class="about-back" type="button"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m14 6-6 6 6 6M8 12h12" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg><span>返回</span></button></div>
    <header class="about-identity"><img class="about-mark" src="${appIcon}" width="64" height="64" alt="XMU Sports 图标"><h3>XMU Sports</h3><p id="about_version"></p></header>
    <section class="preference-card about-section" aria-labelledby="update_title">
      <div class="about-row"><div><h3 id="update_title">应用更新</h3><p id="update_status" role="status" aria-live="polite">${updateHint}</p></div><button class="btn ghost about-update-button" id="update_action" type="button">检查更新</button></div>
      <div class="about-update-detail" hidden>
        <details id="update_notes" hidden><summary>更新说明</summary><div class="release-notes"></div></details>
      </div>
    </section>
    <section class="preference-card about-section" aria-labelledby="repository_title"><div class="about-row"><div><h3 id="repository_title">开源项目</h3><p>MidiAug / xmu-sports</p></div><div class="about-actions"><button class="btn ghost" id="about_copy" type="button">复制地址</button><button class="btn ghost" id="about_repository" type="button">GitHub ↗</button></div></div></section>
    <p class="about-footnote">在本机为本人或已授权账号预约场馆<br>独立开源项目，与厦大体育无官方关联</p>
    <p class="msg" id="about_message" role="status"></p>
  </div>`;
  const query = <T extends HTMLElement = HTMLElement>(selector: string) => panel.querySelector<T>(selector)!;
  query("#about_version").textContent = version;
  query<HTMLButtonElement>(".about-back").onclick = back;
  const message = (text: string) => { if (scope.current()) query("#about_message").textContent = text; };
  const launch = async (url: string) => { try { await open(url); } catch { message("无法打开浏览器，请复制项目地址后访问"); } };
  query<HTMLButtonElement>("#about_repository").onclick = () => { void launch(REPOSITORY); };
  query<HTMLButtonElement>("#about_copy").onclick = async () => { try { await navigator.clipboard.writeText(REPOSITORY); message("已复制项目地址"); } catch { message(`请复制：${REPOSITORY}`); } };
  query<HTMLButtonElement>("#update_action").onclick = () => { void performAppUpdate(); };
  scope.onDispose(watchAppUpdate(state => {
    if (!scope.current()) return;
    const { phase, release } = state;
    const busy = ["checking", "downloading", "installing"].includes(phase);
    const action = query<HTMLButtonElement>("#update_action");
    action.disabled = busy;
    action.dataset.phase = phase;
    action.style.setProperty("--update-progress", `${state.progress}%`);
    let label = "检查更新";
    if (phase === "checking") label = "检查中…";
    else if (phase === "downloading") label = `下载 ${state.progress}%`;
    else if (phase === "installing") label = "正在启动…";
    else if (phase === "downloaded") label = "退出并安装";
    else if (phase === "error") label = state.failedAt === "install" ? "重试安装" : state.failedAt === "download" ? "重新下载" : "重新检查";
    else if (release?.status === "available") label = "下载更新";
    action.textContent = label;
    let status = updateHint;
    if (phase === "checking") status = "正在连接 GitHub…";
    else if (phase === "downloading") status = "正在下载安装包…";
    else if (phase === "downloaded") status = "校验通过，安装时将退出应用";
    else if (phase === "installing") status = "正在校验并启动安装器…";
    else if (phase === "error") status = state.error ?? "更新失败，请重试";
    else if (release?.status === "available") status = `发现新版本 ${release.version}　${(release.size / 1024 / 1024).toFixed(1)} MB`;
    else if (release?.status === "current") status = "当前已是最新版本";
    else if (release?.status === "unpublished") status = "暂未发布正式版本";
    query("#update_status").textContent = status;
    query(".about-update-detail").hidden = !release?.notes || release.status !== "available";
    query("#update_notes").hidden = !release?.notes || release.status !== "available";
    renderReleaseNotes(query(".release-notes"), release?.notes ?? "", url => { void launch(url); });
  }));
}
