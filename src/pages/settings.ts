import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import type { CloseMode, FallbackOrder, ShellPrefs, ShellThemePref } from "../types";
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
import { mountStartupSettings } from "../ui/startup-settings";
import { renderAbout } from "./about";
import { watchAppUpdate } from "../lib/app-update";

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
    .filter(isSportEnabled)
    .map((s) => {
      const on = favoriteIds.has(s.id);
      const cls = [
        "settings-sport-cell",
        on ? "on" : "",
      ]
        .filter(Boolean)
        .join(" ");
      return `<button type="button" class="${cls}" data-fav-sport="${s.id}" aria-pressed="${on}">
        <span class="settings-sport-cell-label">${s.label}</span>
        <span class="settings-sport-check" aria-hidden="true">${SVG.check}</span>
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
      <div class="page-heading"><div><h2>设置</h2><p>外观、常用运动与应用偏好</p></div></div>
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
      <section class="preference-card" id="booking_preferences">
        <div class="preference-card-heading"><h3>预约偏好</h3><p>单次预约与长期计划共用</p></div>
        <div class="settings-cell"><div><label class="settings-cell-title" for="allow_fallback">尝试邻近时段</label><p class="settings-cell-desc">目标时段锁定或已满时，继续尝试候选时段</p></div><label class="switch"><input type="checkbox" id="allow_fallback" aria-label="尝试邻近时段" /></label></div>
        <div class="settings-cell" id="fallback_order_wrap"><div><p class="settings-cell-title">邻近顺序</p><p class="settings-cell-desc">优先仍是你选的时段</p></div><div class="seg-shell"><div class="seg" id="fallback_order_seg"><button type="button" data-order="up_then_down">先更早</button><button type="button" data-order="down_then_up">先更晚</button></div></div></div>
      </section>
      <section class="preference-card" id="startup_settings"></section>
      <section class="preference-card settings-favorites" aria-labelledby="favorite_sports_title">
        <div class="settings-favorites-header"><div><h3 id="favorite_sports_title">常用运动</h3><p id="favorite_sports_summary"></p></div><button type="button" class="favorite-edit" aria-expanded="false" aria-controls="favorite_sports_body"><span class="favorite-edit-label">编辑</span>${SVG.chevron}</button></div>
        <div id="favorite_sports_body" hidden>
          <p class="favorite-hint">选择在预约页显示的运动，至少保留一项，修改自动保存</p>
          ${favoriteGridHtml(favoriteIds)}
          <p class="favorite-unavailable">${sports.filter(s => !isSportEnabled(s)).map(s => s.label).join("、")}暂未支持</p>
          <p class="msg" id="favorite_sports_message" role="status" aria-live="polite"></p>
        </div>
      </section>
      <section class="preference-card settings-diagnostics" aria-labelledby="diagnostics_title">
        <div class="settings-diagnostic-copy"><h3 id="diagnostics_title">诊断与日志</h3><p class="settings-diagnostic-note">分享前请检查日志</p></div>
        <div class="settings-diagnostic-actions"><button type="button" class="btn ghost" id="diag_export_btn" aria-description="保存到下载文件夹">导出日志<span aria-hidden="true">↗</span></button><button type="button" class="btn ghost" id="diag_open_logs_btn">打开文件夹</button></div>

      </section>
      <button type="button" class="preference-card settings-about-entry"><div><strong>关于 XMU Sports</strong><span>版本 ${version}　项目地址与应用更新</span></div><span aria-hidden="true"><svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="m9 5 7 7-7 7" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg></span></button>
      <p class="msg" id="settings_msg"></p>
    </div>
  `;
  void mountStartupSettings(panel.querySelector<HTMLElement>("#startup_settings")!, scope);
  const summary = panel.querySelector<HTMLElement>("#favorite_sports_summary")!;
  const paintSummary = () => { summary.textContent = sports.filter(s => favoriteIds.has(s.id)).map(s => s.label).join("、"); };
  paintSummary();
  const edit = panel.querySelector<HTMLButtonElement>(".favorite-edit")!;
  const favoritesBody = panel.querySelector<HTMLElement>("#favorite_sports_body")!;
  edit.onclick = () => {
    favoritesBody.hidden = !favoritesBody.hidden;
    edit.setAttribute("aria-expanded", String(!favoritesBody.hidden));
    edit.querySelector('.favorite-edit-label')!.textContent = favoritesBody.hidden ? '编辑' : '完成';
  };
  panel.querySelector<HTMLButtonElement>(".settings-about-entry")!.onclick = () => {
    void renderAbout(panel, () => { void renderSettings(panel); });
  };
  scope.onDispose(watchAppUpdate(state => {
    const available = state.release?.status === "available";
    const ready = state.phase === "downloaded" || state.phase === "installing" || state.phase === "error" && state.failedAt === "install";
    panel.querySelector(".settings-about-entry div span")!.textContent = `版本 ${version}　${available ? ready ? "更新已下载，可查看安装" : "有更新，查看更新内容" : "项目地址与应用更新"}`;
  }));

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
  const fallback = panel.querySelector<HTMLInputElement>("#allow_fallback")!;
  const fallbackOrder = panel.querySelector<HTMLElement>("#fallback_order_wrap")!;
  const orderButtons = panel.querySelectorAll<HTMLButtonElement>("[data-order]");
  let savingBookingPrefs = false;
  const paintBookingPrefs = () => {
    fallback.checked = prefs.allow_time_fallback ?? false;
    fallback.setAttribute("aria-disabled", String(savingBookingPrefs));
    fallbackOrder.hidden = !fallback.checked;
    orderButtons.forEach(button => {
      const selected = button.dataset.order === (prefs.time_fallback_order ?? "down_then_up");
      button.classList.toggle("on", selected);
      button.setAttribute("aria-pressed", String(selected));
      button.setAttribute("aria-disabled", String(savingBookingPrefs));
    });
  };
  const saveBookingPrefs = async (patch: Partial<ShellPrefs>) => {
    if (savingBookingPrefs) { paintBookingPrefs(); return; }
    const previous = prefs;
    prefs = {...prefs, ...patch};
    savingBookingPrefs = true;
    paintBookingPrefs();
    try {
      prefs = await updatePrefs(patch);
      setMsg(msg, "", "");
    } catch (error) {
      prefs = previous;
      setMsg(msg, String(error), "err");
    } finally {
      savingBookingPrefs = false;
      if (scope.current()) paintBookingPrefs();
    }
  };
  fallback.onchange = () => { void saveBookingPrefs({allow_time_fallback:fallback.checked}); };
  orderButtons.forEach(button => { button.onclick = () => {
    void saveBookingPrefs({time_fallback_order:button.dataset.order as FallbackOrder});
  }; });
  paintBookingPrefs();

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

  async function persistPrefs(next: Partial<ShellPrefs>, feedback: Element | null = msg) {
    try {
      prefs = await updatePrefs(next);
      log.debug("settings", "save_prefs", {
        close_mode: closeModeFromPrefs(prefs),
        favorites: next.favorite_sport_ids?.length ?? 0,
      });
      setMsg(feedback, "", "");
    } catch (e) {
      log.warn("settings", "save_prefs_failed", { err: String(e) });
      setMsg(feedback, String(e), "err");
      throw e;
    }
  }

  const favoriteMessage = panel.querySelector("#favorite_sports_message");
  const favoriteButtons = panel.querySelectorAll<HTMLButtonElement>("[data-fav-sport]");
  favoriteButtons.forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = btn.dataset.favSport!;
      const sport = sports.find((s) => s.id === id);
      if (!sport || !isSportEnabled(sport) || btn.disabled) return;
      const turningOff = favoriteIds.has(id);
      if (turningOff && favoriteIds.size <= 1) {
        setMsg(favoriteMessage, "至少保留一项常用运动", "err");
        return;
      }
      if (turningOff) favoriteIds.delete(id);
      else favoriteIds.add(id);
      const ordered = resolveFavoriteSportIds([...favoriteIds]);
      favoriteIds = new Set(ordered);
      btn.classList.toggle("on", favoriteIds.has(id));
      btn.setAttribute("aria-pressed", String(favoriteIds.has(id)));
      paintSummary();
      favoriteButtons.forEach(button => { button.disabled = true; });
      favoritesBody.setAttribute("aria-busy", "true");
      try {
        await persistPrefs({
          favorite_sport_ids: ordered,
        }, favoriteMessage);
      } catch {
        if (turningOff) favoriteIds.add(id);
        else favoriteIds.delete(id);
        btn.classList.toggle("on", favoriteIds.has(id));
        btn.setAttribute("aria-pressed", String(favoriteIds.has(id)));
        paintSummary();
      } finally {
        favoriteButtons.forEach(button => { button.disabled = false; });
        favoritesBody.removeAttribute("aria-busy");
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
