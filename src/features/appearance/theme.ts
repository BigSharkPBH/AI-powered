import { useSyncExternalStore } from "react";

export type ThemePreference = "system" | "light" | "dark";
export const THEME_STORAGE_KEY = "ai-assistant.theme";
const listeners = new Set<() => void>();

function parsePreference(value: string | null): ThemePreference {
  return value === "light" || value === "dark" ? value : "system";
}

function readPreference(): ThemePreference {
  try { return parsePreference(window.localStorage.getItem(THEME_STORAGE_KEY)); }
  catch { return "system"; }
}

let preference = readPreference();

function notify(next: ThemePreference) {
  preference = next;
  listeners.forEach((listener) => listener());
}

export function setThemePreference(next: ThemePreference) {
  // 存储不可用时仍允许本次窗口切换主题。
  try { window.localStorage.setItem(THEME_STORAGE_KEY, next); } catch { /* 保留内存偏好。 */ }
  notify(next);
}

export function initializeTheme() {
  notify(readPreference());
  const media = window.matchMedia?.("(prefers-color-scheme: dark)");
  const apply = () => {
    const resolved = preference === "system" ? (media?.matches ? "dark" : "light") : preference;
    document.documentElement.dataset.theme = resolved;
    document.documentElement.style.colorScheme = resolved;
  };
  const onStorage = (event: StorageEvent) => {
    if (event.key === THEME_STORAGE_KEY || event.key === null) notify(parsePreference(event.newValue));
  };
  listeners.add(apply);
  media?.addEventListener("change", apply);
  window.addEventListener("storage", onStorage);
  apply();
  return () => {
    listeners.delete(apply);
    media?.removeEventListener("change", apply);
    window.removeEventListener("storage", onStorage);
  };
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function useThemePreference() {
  return useSyncExternalStore(subscribe, () => preference, () => "system" as const);
}
