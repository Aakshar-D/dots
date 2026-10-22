import "@testing-library/jest-dom/vitest";

import { cleanup, configure } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, beforeEach, vi } from "vitest";

import { backend } from "./backend";
import { installDefaults } from "./fixtures";

// `findBy…` and `waitFor` wait this long; the default second is too short on a busy machine.
configure({ asyncUtilTimeout: 5000 });

// The Tauri bridge, answered by the fake backend. A test file that mocks these modules itself
// (the api tests) takes precedence.
vi.mock("@tauri-apps/api/core", async () => {
  const { backend } = await import("./backend");
  return { invoke: (cmd: string, args?: Record<string, unknown>) => backend.invoke(cmd, args) };
});
vi.mock("@tauri-apps/api/event", async () => {
  const { backend } = await import("./backend");
  return {
    listen: (event: string, listener: (e: { payload: unknown }) => void) =>
      backend.listen(event, listener),
  };
});
vi.mock("@tauri-apps/plugin-dialog", async () => {
  const { backend } = await import("./backend");
  return { open: (options?: Record<string, unknown>) => backend.invoke("dialog_open", options) };
});

// jsdom lacks these; Radix and the toaster call them.
class NoopObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("ResizeObserver", NoopObserver);
window.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener() {},
  removeEventListener() {},
  addListener() {},
  removeListener() {},
  dispatchEvent: () => false,
})) as typeof window.matchMedia;
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.setPointerCapture ??= () => {};
Element.prototype.releasePointerCapture ??= () => {};

beforeEach(() => {
  backend.reset();
  // The home remembers the dot last opened; every test starts without one.
  window.localStorage.clear();
  installDefaults();
});
afterEach(() => {
  cleanup();
  // sonner keeps its toasts outside React and hands the live ones to every new toaster: left
  // alone, a test would show the toasts of the tests before it.
  toast.dismiss();
});
