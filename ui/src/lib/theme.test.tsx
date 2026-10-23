import { readFileSync } from "node:fs";
import path from "node:path";

import { act, render, screen } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Toaster } from "@/components/ui/sonner";
import {
  applyTheme,
  resolvedTheme,
  setThemeChoice,
  startTheme,
  themeChoice,
  THEME_CHOICES,
  THEME_KEY,
} from "@/lib/theme";

// The window's title bar (deep-teal spec section 3). jsdom has no Tauri window.
const setTheme = vi.hoisted(() => vi.fn((_theme: "light" | "dark" | null) => Promise.resolve()));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setTheme }) }));

/// A `prefers-color-scheme: dark` query whose answer the test sets and changes.
function systemTheme(initiallyDark: boolean) {
  const listeners = new Set<() => void>();
  const query = {
    matches: initiallyDark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_type: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener),
  };
  vi.spyOn(window, "matchMedia").mockImplementation(() => query as unknown as MediaQueryList);
  return {
    change(dark: boolean) {
      query.matches = dark;
      for (const listener of [...listeners]) listener();
    },
    listening: () => listeners.size,
  };
}

const html = () => document.documentElement;

beforeEach(() => {
  html().className = "";
  html().style.colorScheme = "";
  setTheme.mockClear();
});
afterEach(() => {
  vi.restoreAllMocks();
});

describe("the theme choice", () => {
  it("is System until one is chosen, and keeps the one chosen under dots.theme", () => {
    expect(themeChoice()).toBe("system");
    setThemeChoice("dark");
    expect(window.localStorage.getItem(THEME_KEY)).toBe("dark");
    expect(themeChoice()).toBe("dark");
    window.localStorage.setItem(THEME_KEY, "purple");
    expect(themeChoice()).toBe("system");
  });

  it("counts storage that cannot be read as System, and still switches for the session", async () => {
    // A fresh module: no choice made in this session yet.
    vi.resetModules();
    const { setThemeChoice, themeChoice } = await import("@/lib/theme");
    systemTheme(false);
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(themeChoice()).toBe("system");
    expect(() => setThemeChoice("dark")).not.toThrow();
    expect(themeChoice()).toBe("dark");
    expect(html()).toHaveClass("dark");
  });

  it("resolves each choice against the system's preference", () => {
    expect(resolvedTheme("system", true)).toBe("dark");
    expect(resolvedTheme("system", false)).toBe("light");
    expect(resolvedTheme("light", true)).toBe("light");
    expect(resolvedTheme("light", false)).toBe("light");
    expect(resolvedTheme("dark", true)).toBe("dark");
    expect(resolvedTheme("dark", false)).toBe("dark");
  });
});

describe("applying the theme", () => {
  it("sets the dark class and the colour scheme on <html>, and the title bar's theme", () => {
    systemTheme(true);
    expect(applyTheme("light")).toBe("light");
    expect(html()).not.toHaveClass("dark");
    expect(html().style.colorScheme).toBe("light");
    expect(setTheme).toHaveBeenLastCalledWith("light");

    expect(applyTheme("system")).toBe("dark");
    expect(html()).toHaveClass("dark");
    expect(html().style.colorScheme).toBe("dark");
    // System lets Windows choose the title bar.
    expect(setTheme).toHaveBeenLastCalledWith(null);
  });

  it("ignores a title bar that cannot be themed: the page is themed anyway", async () => {
    systemTheme(false);
    setTheme.mockImplementationOnce(() => Promise.reject(new Error("not allowed")));
    expect(applyTheme("dark")).toBe("dark");
    expect(html()).toHaveClass("dark");
    await act(async () => {});
  });

  it("follows the system while System is chosen, and only then", () => {
    const system = systemTheme(false);
    const stop = startTheme();
    expect(html()).not.toHaveClass("dark");
    system.change(true);
    expect(html()).toHaveClass("dark");

    setThemeChoice("light");
    expect(html()).not.toHaveClass("dark");
    system.change(false);
    system.change(true);
    expect(html()).not.toHaveClass("dark");

    stop();
    expect(system.listening()).toBe(0);
  });
});

describe("the script that themes the page before the first paint", () => {
  const source = readFileSync(path.resolve(import.meta.dirname, "../../public/theme-boot.js"), "utf8");

  /// Runs public/theme-boot.js against this storage and system preference, as index.html does.
  function boot(storage: Pick<Storage, "getItem"> | null, prefersDark: boolean) {
    const root = document.createElement("html");
    const run = new Function("window", "document", source);
    run(
      { localStorage: storage, matchMedia: () => ({ matches: prefersDark }) },
      { documentElement: root },
    );
    return root;
  }

  it("picks the theme the app would, for every choice and system preference", () => {
    for (const stored of [null, "system", "light", "dark", "purple"]) {
      for (const prefersDark of [false, true]) {
        const root = boot({ getItem: () => stored }, prefersDark);
        const choice = THEME_CHOICES.find((c) => c === stored) ?? "system";
        const want = resolvedTheme(choice, prefersDark);
        expect(root.classList.contains("dark"), `${stored} ${prefersDark}`).toBe(want === "dark");
        expect(root.style.colorScheme).toBe(want);
      }
    }
  });

  it("counts storage that cannot be read as System", () => {
    const broken = {
      getItem: () => {
        throw new Error("blocked");
      },
    };
    expect(boot(broken, true)).toHaveClass("dark");
    expect(boot(null, false)).not.toHaveClass("dark");
  });

  it("is what index.html loads, and <html> no longer starts dark", () => {
    const page = readFileSync(path.resolve(import.meta.dirname, "../../index.html"), "utf8");
    expect(page).toContain('<script src="/theme-boot.js"></script>');
    expect(page).not.toContain('class="dark"');
  });
});

describe("the toasts", () => {
  it("follow the theme on screen", async () => {
    systemTheme(false);
    setThemeChoice("dark");
    render(<Toaster />);
    act(() => {
      toast("Saved");
    });
    await screen.findByText("Saved");
    const toaster = () => document.querySelector("[data-sonner-toaster]");
    expect(toaster()).toHaveAttribute("data-sonner-theme", "dark");
    act(() => setThemeChoice("light"));
    expect(toaster()).toHaveAttribute("data-sonner-theme", "light");
  });
});
