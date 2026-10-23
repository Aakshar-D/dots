import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSyncExternalStore } from "react";

/// The person's choice of theme (deep-teal spec section 3), kept per device in localStorage.
export type ThemeChoice = "system" | "light" | "dark";
export type Theme = "light" | "dark";

export const THEME_KEY = "dots.theme";
export const THEME_CHOICES: readonly ThemeChoice[] = ["system", "light", "dark"];
const DARK_QUERY = "(prefers-color-scheme: dark)";

/// The choice made in this session, for when storage cannot be read.
let sessionChoice: ThemeChoice = "system";
/// What re-renders when the theme on screen changes (the toaster).
const listeners = new Set<() => void>();

const parseChoice = (stored: string | null): ThemeChoice =>
  THEME_CHOICES.find((choice) => choice === stored) ?? "system";

/// The choice kept on this device: System when there is none, or it is not one of the three.
/// Storage that cannot be read counts as System, or as what was chosen in this session.
export function themeChoice(): ThemeChoice {
  try {
    return parseChoice(window.localStorage.getItem(THEME_KEY));
  } catch {
    return sessionChoice;
  }
}

/// The theme a choice shows. `public/theme-boot.js` applies the same rule before the first
/// paint.
export function resolvedTheme(choice: ThemeChoice, prefersDark: boolean): Theme {
  return choice === "system" ? (prefersDark ? "dark" : "light") : choice;
}

function prefersDark(): boolean {
  return typeof window.matchMedia === "function" && window.matchMedia(DARK_QUERY).matches;
}

/// Shows `choice` (by default the one kept): the `dark` class and the colour scheme on <html>,
/// the window's title bar (System lets Windows choose), and whatever follows the theme.
export function applyTheme(choice: ThemeChoice = themeChoice()): Theme {
  const theme = resolvedTheme(choice, prefersDark());
  const root = document.documentElement;
  root.classList.toggle("dark", theme === "dark");
  root.style.colorScheme = theme;
  try {
    getCurrentWindow()
      .setTheme(choice === "system" ? null : choice)
      .catch(() => {});
  } catch {
    // No Tauri window (a test, a browser): the page is themed anyway.
  }
  for (const listener of listeners) listener();
  return theme;
}

/// Keeps `choice` on this device and shows it at once. A choice that cannot be kept still
/// holds for this session.
export function setThemeChoice(choice: ThemeChoice): void {
  sessionChoice = choice;
  try {
    window.localStorage.setItem(THEME_KEY, choice);
  } catch {
    // Kept for this session only.
  }
  applyTheme(choice);
}

/// Shows the kept choice and, while it is System, follows Windows when its theme changes.
/// Returns what stops following.
export function startTheme(): () => void {
  applyTheme();
  if (typeof window.matchMedia !== "function") return () => {};
  const query = window.matchMedia(DARK_QUERY);
  const follow = () => {
    if (themeChoice() === "system") applyTheme("system");
  };
  query.addEventListener("change", follow);
  return () => query.removeEventListener("change", follow);
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const shownTheme = (): Theme =>
  document.documentElement.classList.contains("dark") ? "dark" : "light";

/// The theme on screen, for what React draws (the toasts).
export function useResolvedTheme(): Theme {
  return useSyncExternalStore(subscribe, shownTheme);
}
