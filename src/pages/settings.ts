import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import type { CloseMode, ShellPrefs, ShellThemePref } from "../types";
import { closeModeFromPrefs, withCloseMode } from "../lib/close-prefs";
import {
  isSportEnabled,
  resolveFavoriteSportIds,
  sports,
} from "../lib/catalog";
import { setMsg } from "../lib/msg";
import { log } from "../lib/log";
import { applyTheme, currentThemePref } from "../lib/theme";
import { SVG } from "../ui/icons";
import { mountShellSelect } from "../ui/select";
import { mountPage } from "../lib/page-lifecycle";
import { updatePrefs } from "../lib/prefs-store";
import { renderAbout } from "./about";

function themeCubesHtml(): string {
  const pref = currentThemePref();
  const items: { id: ShellThemePref; label: string; icon: string }[] = [
    { id: "light", label: "浅色", icon: SVG.light },
    { id: "dark", label: "深色", icon: SVG.dark },
    { id: "system", label: "跟随系统", icon: SVG.system },
  ];
  return `<div class="settings-theme-cubes" role="radiogroup" aria-label="主题">
    ${items
      .map(
        (it) => `<button type="button" class="settings-theme-cube${
          pref === it.id ? " on" : ""
        }" data-theme="${it.id}" role="radio" aria-checked="${pref === it.id}">
          <span class="theme-preview theme-preview-${it.id}" aria-hidden="true"><i></i><i></i><i></i></span>
          <span class="theme-option-label">${it.icon}${it.label}<span class="theme-option-check">${SVG.check}</span></span>
        </button>`,
      )
      .join("")}
  </div>`;
}

const CLOSE_MODE_OPTIONS: { value: CloseMode; label: string }[] = [
  { value: "ask", label: "每次询问" },
  { value: "tray", label: "最小化到托盘" },
  { value: "quit", label: "退出应用" },
];

function favoriteGridHtml(favoriteIds: Set<string>): string {
  const cells = sports
    .map((s) => {
      const ok = isSportEnabled(s);
      const on = ok && favoriteIds.has(s.id);
      const cls = [
        "settings-sport-cell",
        on ? "on" : "",
        ok ? "" : "disabled",
      ]
        .filter(Boolean)
        .join(" ");
      return `<button type="button" class="${cls}" data-fav-sport="${s.id}" ${
        ok ? "" : "disabled aria-disabled=\"true\""
      } aria-pressed="${on}" title="${ok ? s.label : `${s.label}（暂未支持）`}">
        <span class="settings-sport-cell-label">${s.label}</span>
        ${ok ? `<span class="settings-sport-check" aria-hidden="true">${SVG.check}</span>` : ""}
        ${ok ? "" : '<span class="settings-sport-cell-hint">暂未支持</span>'}
      </button>`;
    })
    .join("");
  return `<div class="settings-sport-grid" role="group" aria-label="常用运动">${cells}</div>`;
}

export async function renderSettings(panel: Element) {
  const scope = mountPage(panel);
  let prefs: ShellPrefs = {
    close_to_tray: true,
    close_pref_set: false,
    close_remember_default_used: false,
    favorite_sport_ids: [],
    venue_slot_prefs: {},
  };
  try {
    prefs = await invoke<ShellPrefs>("get_shell_prefs");
  } catch {
    /* ignore */
  }
  let closeMode = closeModeFromPrefs(prefs);
  let favoriteIds = new Set(resolveFavoriteSportIds(prefs.favorite_sport_ids));

  let version = "0.1.0";
  try {
    version = await getVersion();
  } catch {
    /* ignore */
  }

  if (!scope.current()) return;
  panel.innerHTML = `
    <div class="page-stack settings-page">
      <div class="page-heading"><div><h2>设置</h2><p>常用运动、外观与应用偏好</p></div></div>
      <section class="preference-card">
        <div class="preference-card-heading"><h3>常用运动</h3><p>选择要在预约页显示的运动，至少保留一项</p></div>
        <div id="favorite_sports_body">
          ${favoriteGridHtml(favoriteIds)}
        </div>
      </section>
      <section class="preference-card">
        <div class="preference-card-heading"><h3>外观与窗口</h3><p>按照你的使用习惯调整</p></div>
        <div id="settings_theme_host">${themeCubesHtml()}</div>
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">关闭窗口时</p>
              <p class="settings-cell-desc">定时预约时，建议保留在托盘运行</p>
            </div>
            <div id="close_mode_host"></div>
          </div>
      </section>
      <details class="preference-card settings-diagnostics">
        <summary><div><strong>诊断与日志</strong><span>遇到问题时，导出日志以便排查</span></div><span class="settings-disclosure" aria-hidden="true">⌄</span></summary>
        <div class="settings-diagnostics-body">
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">导出诊断日志</p>
              <p class="settings-cell-desc">保存到「下载」文件夹。日志可能包含旧版本历史记录，分享前请检查</p>
            </div>
            <button type="button" class="btn ghost" id="diag_export_btn">导出</button>
          </div>
          <div class="settings-cell">
            <div>
              <p class="settings-cell-title">日志文件夹</p>
              <p class="settings-cell-desc">查看此电脑保存的运行日志</p>
            </div>
            <button type="button" class="btn ghost" id="diag_open_logs_btn">打开</button>
          </div>
        </div>
      </details>
      <button type="button" class="preference-card settings-about-entry"><div><strong>关于 XMU Sports</strong><span>版本 ${version}　项目地址与应用更新</span></div><span aria-hidden="true">›</span></button>
      <p class="msg" id="settings_msg"></p>
    </div>
  `;
  panel.querySelector<HTMLButtonElement>(".settings-about-entry")!.onclick = () => {
    void renderAbout(panel, () => { void renderSettings(panel); });
  };

  panel.querySelectorAll("[data-theme]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const next = (btn as HTMLButtonElement).dataset.theme as ShellThemePref;
      applyTheme(next);
      panel.querySelectorAll("[data-theme]").forEach((b) => {
        const on = (b as HTMLButtonElement).dataset.theme === next;
        b.classList.toggle("on", on);
        b.setAttribute("aria-checked", String(on));
      });
    });
  });

  const msg = panel.querySelector("#settings_msg");

  const exportBtn = panel.querySelector("#diag_export_btn") as HTMLButtonElement;
  const openLogsBtn = panel.querySelector("#diag_open_logs_btn") as HTMLButtonElement;
  exportBtn.addEventListener("click", async () => {
    if (exportBtn.disabled) return;
    exportBtn.disabled = true;
    setMsg(msg, "正在导出…", "");
    try {
      const path = await invoke<string>("export_diagnostic_logs");
      const name = path.replace(/^.*[/\\]/, "");
      log.info("settings", "diag_export_ok", { name });
      setMsg(msg, `已导出到下载：${name}`, "ok");
    } catch (e) {
      log.warn("settings", "diag_export_failed", { err: String(e) });
      setMsg(msg, String(e), "err");
    } finally {
      exportBtn.disabled = false;
    }
  });

  openLogsBtn.addEventListener("click", async () => {
    if (openLogsBtn.disabled) return;
    openLogsBtn.disabled = true;
    try {
      await invoke<string>("open_app_log_dir");
      log.info("settings", "diag_open_logs_ok");
      setMsg(msg, "已打开日志文件夹", "ok");
    } catch (e) {
      log.warn("settings", "diag_open_logs_failed", { err: String(e) });
      setMsg(msg, String(e), "err");
    } finally {
      openLogsBtn.disabled = false;
    }
  });

  async function persistPrefs(next: Partial<ShellPrefs>) {
    try {
      prefs = await updatePrefs(next);
      log.debug("settings", "save_prefs", {
        close_mode: closeModeFromPrefs(prefs),
        favorites: next.favorite_sport_ids?.length ?? 0,
      });
      setMsg(msg, "", "");
    } catch (e) {
      log.warn("settings", "save_prefs_failed", { err: String(e) });
      setMsg(msg, String(e), "err");
      throw e;
    }
  }

  panel.querySelectorAll<HTMLButtonElement>("[data-fav-sport]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = btn.dataset.favSport!;
      const sport = sports.find((s) => s.id === id);
      if (!sport || !isSportEnabled(sport) || btn.disabled) return;
      const turningOff = favoriteIds.has(id);
      if (turningOff && favoriteIds.size <= 1) {
        setMsg(msg, "至少保留一项常用运动", "err");
        return;
      }
      if (turningOff) favoriteIds.delete(id);
      else favoriteIds.add(id);
      const ordered = resolveFavoriteSportIds([...favoriteIds]);
      favoriteIds = new Set(ordered);
      btn.classList.toggle("on", favoriteIds.has(id));
      btn.setAttribute("aria-pressed", String(favoriteIds.has(id)));
      try {
        await persistPrefs({
          favorite_sport_ids: ordered,
        });
      } catch {
        if (turningOff) favoriteIds.add(id);
        else favoriteIds.delete(id);
        btn.classList.toggle("on", favoriteIds.has(id));
        btn.setAttribute("aria-pressed", String(favoriteIds.has(id)));
      }
    });
  });

  const closeHost = panel.querySelector("#close_mode_host") as HTMLElement;
  mountShellSelect(
    closeHost,
    CLOSE_MODE_OPTIONS,
    closeMode,
    async (value) => {
      closeMode = value as CloseMode;
      try {
        const cur = withCloseMode(prefs, closeMode);
        await persistPrefs({ close_to_tray: cur.close_to_tray, close_pref_set: cur.close_pref_set,
          close_remember_default_used: cur.close_remember_default_used });
      } catch (e) {
        log.warn("settings", "save_prefs_failed", { err: String(e) });
        setMsg(msg, String(e), "err");
      }
    },
  );
}
