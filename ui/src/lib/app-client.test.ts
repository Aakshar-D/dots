// The client Dots puts into an app's page (apps/dots-app/src/app_client.js), run in a context of
// its own with a stand-in for the page's window.
import { runInNewContext } from "node:vm";

import { describe, expect, it } from "vitest";

import client from "../../../apps/dots-app/src/app_client.js?raw";

type Dots = {
  call: (tool: string, input?: unknown) => Promise<unknown>;
  onChange: (cb: () => void) => () => void;
  onTheme: (cb: (mode: string) => void) => () => void;
};

/// A page with the client loaded: what it set on `window`, a way to deliver a `message` event,
/// the window that framed it, and its root element's style.
function page() {
  const listeners: ((event: unknown) => void)[] = [];
  const style = new Map<string, string>();
  const root = {
    style: { setProperty: (name: string, value: string) => style.set(name, value), colorScheme: "" },
    dataset: {} as Record<string, string>,
  };
  const parent = {};
  const window: Record<string, unknown> = {
    parent,
    addEventListener: (type: string, listener: (event: unknown) => void) => {
      if (type === "message") listeners.push(listener);
    },
  };
  runInNewContext(client, { window, document: { documentElement: root }, console });
  const deliver = (event: unknown) => {
    for (const listener of listeners) listener(event);
  };
  return { window, dots: window.dots as Dots, deliver, parent, root, style };
}

/// The next message that arrives on `port`.
function next(port: MessagePort): Promise<unknown> {
  return new Promise((resolve) => {
    port.onmessage = (e: MessageEvent) => resolve(e.data);
  });
}

function connect(p: ReturnType<typeof page>) {
  const { port1, port2 } = new MessageChannel();
  p.deliver({ source: p.parent, data: { type: "dots:connect" }, ports: [port2] });
  return port1;
}

describe("the client in an app's page", () => {
  it("sends calls through the port once it has one, and answers with the result", async () => {
    const p = page();
    const listed = p.dots.call("list_expenses", { month: "2026-10" });
    const port = connect(p);
    const asked = await next(port);
    expect(asked).toEqual({
      type: "call",
      id: 1,
      tool: "list_expenses",
      input: { month: "2026-10" },
    });
    port.postMessage({ type: "result", id: 1, ok: true, result: [{ item: "Ink" }] });
    await expect(listed).resolves.toEqual([{ item: "Ink" }]);

    const refused = p.dots.call("drop_all");
    expect(await next(port)).toEqual({ type: "call", id: 2, tool: "drop_all", input: {} });
    port.postMessage({ type: "result", id: 2, ok: false, error: "this app has no tool drop_all" });
    await expect(refused).rejects.toThrow("this app has no tool drop_all");
    port.close();
  });

  it("takes one port, and only from the window that framed it", async () => {
    const p = page();
    const stranger = new MessageChannel();
    p.deliver({ source: {}, data: { type: "dots:connect" }, ports: [stranger.port2] });
    const pending = p.dots.call("list_expenses");
    const port = connect(p);
    expect(await next(port)).toMatchObject({ type: "call", id: 1 });
    const second = new MessageChannel();
    p.deliver({ source: p.parent, data: { type: "dots:connect" }, ports: [second.port2] });
    second.port1.postMessage({ type: "result", id: 1, ok: true, result: "forged" });
    port.postMessage({ type: "result", id: 1, ok: true, result: "real" });
    await expect(pending).resolves.toBe("real");
    for (const q of [port, stranger.port1, second.port1]) q.close();
  });

  it("puts the theme's colours on the root and tells the page of changes", async () => {
    const p = page();
    const port = connect(p);
    const modes: string[] = [];
    p.dots.onTheme((mode) => modes.push(mode));
    let changes = 0;
    const stop = p.dots.onChange(() => (changes += 1));
    const heard = new Promise<void>((resolve) => p.dots.onChange(() => resolve()));
    port.postMessage({ type: "theme", mode: "dark", tokens: { background: "#0b1514" } });
    port.postMessage({ type: "changed" });
    await heard;
    expect(p.style.get("--dots-background")).toBe("#0b1514");
    expect(p.root.dataset.theme).toBe("dark");
    expect(p.root.style.colorScheme).toBe("dark");
    expect(modes).toEqual(["dark"]);
    expect(changes).toBe(1);
    stop();
    port.close();
  });

  it("cannot be replaced by the page", () => {
    const p = page();
    expect(() => runInNewContext('"use strict"; window.dots = {};', { window: p.window })).toThrow();
    expect(p.window.dots).toBe(p.dots);
    expect(Object.isFrozen(p.dots)).toBe(true);
  });
});
