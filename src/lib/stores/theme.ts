import { get, writable } from "svelte/store";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { TAURI_ENV } from "../invoker";

// Light / dark theme of the main window. Preview windows are always dark and
// do not read this.

export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "theme";

function readPreference(): ThemePreference {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved === "light" || saved === "dark" ? saved : "system";
}

export const themePreference = writable<ThemePreference>(readPreference());

export function setThemePreference(preference: ThemePreference) {
  localStorage.setItem(STORAGE_KEY, preference);
  themePreference.set(preference);
}

/**
 * Keep `<html class="dark">` in sync with the preference. Call once, before
 * mounting the main window, so the first paint already has the right theme.
 */
export function initTheme() {
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () => {
    const preference = get(themePreference);
    const dark =
      preference === "dark" || (preference === "system" && systemDark.matches);
    document.documentElement.classList.toggle("dark", dark);
  };

  systemDark.addEventListener("change", apply);
  themePreference.subscribe((preference) => {
    apply();
    // The native appearance drives `prefers-color-scheme`, scrollbars and
    // form popups; null lets the window follow the system again.
    if (TAURI_ENV) {
      void getCurrentWindow()
        .setTheme(preference === "system" ? null : preference)
        .catch((error) => console.error("设置窗口主题失败", error));
    }
  });
}
