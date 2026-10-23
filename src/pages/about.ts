import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import { mountPage } from "../lib/page-lifecycle";
import { watchAppUpdate, checkAppUpdate, downloadAppUpdate } from "../lib/app-update";

const REPOSITORY = "https://github.com/MidiAug/xmu-sports";
export async function renderAbout(panel: Element, back: () => void) {
  const scope = mountPage(panel);
  let version = "版本暂不可用";
  try { version = `版本 ${await getVersion()}`; } catch { /* Browser preview. */ }
  if (!scope.current()) return;
  panel.closest<HTMLElement>(".content")?.scrollTo({ top: 0 });
  panel.innerHTML = `<div class="page-stack settings-page about-page">
    <div class="page-heading"><div><button class="about-back" type="button">‹ 返回设置</button><h2>关于</h2></div></div>
    <header class="about-identity"><div class="about-mark" aria-hidden="true">X<span>MU</span></div><h3>XMU Sports</h3><p id="about_version"></p><p class="about-tagline">让场馆预约，从容一点</p></header>
    <section class="preference-card about-section" aria-labelledby="update_title">
      <div class="about-row"><div><h3 id="update_title">应用更新</h3><p id="update_status" role="status" aria-live="polite">手动获取 GitHub 上的最新正式版本</p></div><button class="btn ghost" id="update_check" type="button">检查更新</button></div>
      <div class="about-update-detail" hidden>
        <progress id="update_progress" max="100" value="0" aria-label="安装包下载进度" hidden></progress>
        <details id="update_notes" hidden><summary>更新说明</summary><pre></pre></details>
        <p id="update_help" hidden></p>
        <div class="about-actions"><button class="btn primary" id="update_download" type="button" hidden>下载更新</button><button class="btn ghost" id="update_folder" type="button" hidden>打开文件夹</button><button class="about-link" id="update_releases" type="button">查看发布页 ↗</button></div>
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
  query<HTMLButtonElement>("#update_releases").onclick = () => { void launch(`${REPOSITORY}/releases`); };
  query<HTMLButtonElement>("#about_copy").onclick = async () => { try { await navigator.clipboard.writeText(REPOSITORY); message("已复制项目地址"); } catch { message(`请复制：${REPOSITORY}`); } };
  query<HTMLButtonElement>("#update_check").onclick = () => { void checkAppUpdate(); };
  query<HTMLButtonElement>("#update_download").onclick = () => { void downloadAppUpdate(); };
  query<HTMLButtonElement>("#update_folder").onclick = async () => { try { await invoke("open_update_folder"); } catch (e) { message(String(e)); } };
  scope.onDispose(watchAppUpdate(state => {
    if (!scope.current()) return;
    const { phase, release } = state;
    const busy = phase === "checking" || phase === "downloading";
    const check = query<HTMLButtonElement>("#update_check");
    check.disabled = busy; check.textContent = phase === "checking" ? "检查中…" : "检查更新";
    let status = "手动获取 GitHub 上的最新正式版本";
    if (phase === "checking") status = "正在连接 GitHub…";
    else if (phase === "downloading") status = `正在下载 ${state.progress}%`;
    else if (phase === "downloaded") status = "下载完成，校验通过";
    else if (phase === "error") status = state.error ?? "更新失败，请重试";
    else if (release?.status === "available") status = `发现新版本 ${release.version}　${(release.size / 1024 / 1024).toFixed(1)} MB`;
    else if (release?.status === "current") status = "当前已是最新版本";
    else if (release?.status === "unpublished") status = "暂未发布正式版本";
    query("#update_status").textContent = status;
    query(".about-update-detail").hidden = phase === "idle";
    query<HTMLProgressElement>("#update_progress").hidden = phase !== "downloading";
    query<HTMLProgressElement>("#update_progress").value = state.progress;
    query("#update_notes").hidden = !release?.notes || release.status !== "available";
    query("#update_notes pre").textContent = release?.notes ?? "";
    query("#update_download").hidden = busy || phase === "downloaded" || release?.status !== "available";
    query("#update_download").textContent = phase === "error" ? "重新下载" : "下载更新";
    query("#update_folder").hidden = phase !== "downloaded";
    query("#update_help").hidden = phase !== "downloaded";
    query("#update_help").textContent = `安装包已保存：${state.path ?? ""}\n请结束预约并退出应用后，再打开安装包。`;
  }));
}
