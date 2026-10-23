import type { ShellThemePref, ResolvedTheme } from "../types";
import { setThemePref, themePref } from "../state";

export const THEME_KEY = "xmu.shellTheme";

export function readThemePref(): ShellThemePref {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* ignore */
  }
  return "system";
}

export function osPrefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)")?.matches ?? true;
}

export function resolveTheme(pref: ShellThemePref): ResolvedTheme {
  if (pref === "light") return "light";
  if (pref === "dark") return "dark";
  return osPrefersDark() ? "dark" : "light";
}

export function applyTheme(pref: ShellThemePref) {
  setThemePref(pref);
  const resolved = resolveTheme(pref);
  document.documentElement.setAttribute("data-shell-theme", resolved);
  document.documentElement.style.colorScheme = resolved;
  document.documentElement.style.background =
    resolved === "dark" ? "rgb(21, 21, 23)" : "rgb(245, 245, 247)";
  try {
    localStorage.setItem(THEME_KEY, pref);
  } catch {
    /* ignore */
  }
}

export function currentThemePref(): ShellThemePref {
  return themePref;
}
