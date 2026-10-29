import { invoke } from "@tauri-apps/api/core";
import { updatePrefs } from "../lib/prefs-store";
import type { PageScope } from "../lib/page-lifecycle";

type Startup = { supported: boolean; enabled: boolean; hidden: boolean };
export async function mountStartupSettings(host: HTMLElement, scope: PageScope) {
  host.innerHTML = `<div class="preference-card-heading"><h3>启动与后台</h3><p>保留在后台，自动执行长期计划</p></div>
    <label class="settings-cell"><div><p class="settings-cell-title">登录 Windows 后启动</p><p class="settings-cell-desc" id="startup_hint">正在读取系统设置</p></div><span class="switch"><input type="checkbox" id="startup_enabled" disabled></span></label>
    <label class="settings-cell"><div><p class="settings-cell-title">自动启动时进入托盘</p><p class="settings-cell-desc">手动打开应用仍会显示主窗口</p></div><span class="switch"><input type="checkbox" id="startup_hidden" disabled></span></label><p class="msg" id="startup_msg" role="status"></p>`;
  const enabled = host.querySelector<HTMLInputElement>("#startup_enabled")!;
  const hidden = host.querySelector<HTMLInputElement>("#startup_hidden")!;
  const message = host.querySelector<HTMLElement>("#startup_msg")!;
  try {
    const status = await invoke<Startup>("get_startup");
    if (!scope.current()) return;
    enabled.checked = status.enabled; hidden.checked = status.hidden;
    enabled.disabled = !status.supported; hidden.disabled = false;
    host.querySelector("#startup_hint")!.textContent = status.supported ? "电脑需保持唤醒和联网" : "开发版不注册启动项，请在安装版设置";
    enabled.onchange = async () => {
      enabled.disabled = true;
      try { const saved = await invoke<Startup>("set_startup", { enabled: enabled.checked }); enabled.checked = saved.enabled; message.textContent = ""; }
      catch (e) { enabled.checked = !enabled.checked; message.textContent = String(e); }
      finally { enabled.disabled = false; }
    };
    hidden.onchange = async () => {
      hidden.disabled = true;
      try { await updatePrefs({ autostart_hidden: hidden.checked }); message.textContent = ""; }
      catch (e) { hidden.checked = !hidden.checked; message.textContent = String(e); }
      finally { hidden.disabled = false; }
    };
  } catch (e) { if (scope.current()) message.textContent = String(e); }
}
