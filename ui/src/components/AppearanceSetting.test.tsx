import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { THEME_KEY } from "@/lib/theme";
import { backend } from "@/test/backend";
import { renderApp } from "@/test/render";

// The window's title bar (deep-teal spec section 3). jsdom has no Tauri window.
const setTheme = vi.hoisted(() => vi.fn((_theme: "light" | "dark" | null) => Promise.resolve()));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ setTheme }) }));

const appearance = async () =>
  within(await screen.findByRole("group", { name: "Appearance" }));

beforeEach(() => {
  document.documentElement.className = "";
  setTheme.mockClear();
});

describe("Settings, Appearance", () => {
  it("offers System, Light and Dark, with System chosen at first", async () => {
    renderApp("/settings");
    const group = await appearance();
    expect(group.getByRole("radio", { name: "System" })).toBeChecked();
    expect(group.getByRole("radio", { name: "Light" })).not.toBeChecked();
    expect(group.getByRole("radio", { name: "Dark" })).not.toBeChecked();
  });

  it("applies a choice at once, to the page and the title bar, and keeps it", async () => {
    const first = renderApp("/settings");
    await first.user.click((await appearance()).getByRole("radio", { name: "Dark" }));
    expect(document.documentElement).toHaveClass("dark");
    expect(setTheme).toHaveBeenLastCalledWith("dark");
    expect(window.localStorage.getItem(THEME_KEY)).toBe("dark");
    first.unmount();

    const again = renderApp("/settings");
    const group = await appearance();
    expect(group.getByRole("radio", { name: "Dark" })).toBeChecked();
    await again.user.click(group.getByRole("radio", { name: "Light" }));
    expect(document.documentElement).not.toHaveClass("dark");
    expect(setTheme).toHaveBeenLastCalledWith("light");
    await again.user.click(group.getByRole("radio", { name: "System" }));
    expect(setTheme).toHaveBeenLastCalledWith(null);
    expect(window.localStorage.getItem(THEME_KEY)).toBe("system");
  });

  it("is not part of Save: choosing saves no settings", async () => {
    const { user } = renderApp("/settings");
    await user.click((await appearance()).getByRole("radio", { name: "Light" }));
    expect(backend.callsTo("update_settings")).toEqual([]);
  });
});
