// The client Dots puts into an app's page (apps/dots-app/src/app_client.js), run in a context of
// its own with a stand-in for the page's window.
import { Blob } from "node:buffer";
import { TextEncoder } from "node:util";
import { runInNewContext } from "node:vm";

import { describe, expect, it } from "vitest";

import client from "../../../apps/dots-app/src/app_client.js?raw";

type Dots = {
  call: (tool: string, input?: unknown) => Promise<unknown>;
  openRun: (runId: string) => Promise<unknown>;
  saveFile: (name: string, base64: string) => Promise<unknown>;
  onChange: (cb: () => void) => () => void;
  onTheme: (cb: (mode: string) => void) => () => void;
};

/// A page with the client loaded: what it set on `window`, a way to deliver a `message` event,
/// the window that framed it, and its root element's style.
function page(more: Record<string, unknown> = {}) {
  const listeners: ((event: unknown) => void)[] = [];
  const clicks: ((event: unknown) => void)[] = [];
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
      if (type === "click") clicks.push(listener);
    },
    ...more,
  };
  runInNewContext(client, { window, document: { documentElement: root }, console });
  const deliver = (event: unknown) => {
    for (const listener of listeners) listener(event);
  };
  /// A click that reached the window, on `target`.
  const click = (event: unknown) => {
    for (const listener of clicks) listener(event);
  };
  return { window, dots: window.dots as Dots, deliver, click, parent, root, style };
}

/// A link as the client reads one: its attributes, and whether it is on the page.
class Link {
  isConnected = false;
  /// How often the browser's own `click` ran.
  clicked = 0;
  constructor(private attributes: Record<string, string>) {}
  getAttribute(name: string): string | null {
    return this.attributes[name] ?? null;
  }
  closest(): Link {
    return this;
  }
  click(): void {
    this.clicked += 1;
  }
}

/// A page whose window has what a browser's has for files: `URL` (it numbers the addresses it
/// makes), `Blob`, links, `btoa`, and a `window.open` that counts its calls.
function filePage() {
  let made = 0;
  const revoked: string[] = [];
  const opened: unknown[] = [];
  // A class of its own, so that the client's change to `click` stays in this page.
  class Anchor extends Link {}
  const p = page({
    URL: {
      createObjectURL: () => `blob:app/${++made}`,
      revokeObjectURL: (url: string) => revoked.push(url),
    },
    Blob,
    HTMLAnchorElement: Anchor,
    TextEncoder,
    btoa,
    open: (url: unknown) => {
      opened.push(url);
      return "a window";
    },
  });
  type Urls = { createObjectURL: (o: unknown) => string; revokeObjectURL: (u: string) => void };
  const open = p.window.open as (url: string) => unknown;
  return { ...p, Anchor, urls: p.window.URL as Urls, open, revoked, opened };
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

  it("asks the host to open a run, and settles with its answer", async () => {
    const p = page();
    const port = connect(p);
    const opened = p.dots.openRun("r1");
    expect(await next(port)).toEqual({ type: "open_run", id: 1, run_id: "r1" });
    port.postMessage({ type: "result", id: 1, ok: true, result: null });
    await expect(opened).resolves.toBeNull();
    const refused = p.dots.openRun("r2");
    await next(port);
    port.postMessage({ type: "result", id: 2, ok: false, error: "the app started no run r2" });
    await expect(refused).rejects.toThrow("the app started no run r2");
  });

  it("hands Dots a file to save, and answers whether the person saved it", async () => {
    const p = page();
    const port = connect(p);
    const saved = p.dots.saveFile("INV-1 - Acme.pdf", "JVBERi0xLjQK");
    expect(await next(port)).toEqual({
      type: "save_file",
      id: 1,
      name: "INV-1 - Acme.pdf",
      data: "JVBERi0xLjQK",
    });
    port.postMessage({ type: "result", id: 1, ok: true, result: true });
    await expect(saved).resolves.toBe(true);
    const cancelled = p.dots.saveFile("a.pdf", "");
    await next(port);
    port.postMessage({ type: "result", id: 2, ok: true, result: false });
    await expect(cancelled).resolves.toBe(false);
    port.close();
  });

  it("saves the file of a download link the page made from a blob", async () => {
    const p = filePage();
    const port = connect(p);
    const url = p.urls.createObjectURL(new Blob(["%PDF-1.4\n"], { type: "application/pdf" }));
    expect(url).toBe("blob:app/1");
    // The usual way: a link that is not on the page, clicked from script, its address revoked
    // at once.
    const link = new p.Anchor({ href: url, download: "INV-1 - Acme.pdf" });
    link.click();
    p.urls.revokeObjectURL(url);
    expect(await next(port)).toEqual({
      type: "save_file",
      id: 1,
      name: "INV-1 - Acme.pdf",
      data: btoa("%PDF-1.4\n"),
    });
    // The browser's own click, which the frame would refuse, was not tried.
    expect(link.clicked).toBe(0);
    expect(p.revoked).toEqual([url]);

    // A link on the page: the click reaches the window, and is taken from the browser.
    const second = p.urls.createObjectURL(new Blob(["a,b\n1,2\n"], { type: "text/csv" }));
    let prevented = false;
    const event = {
      defaultPrevented: false,
      target: new p.Anchor({ href: second, download: "" }),
      preventDefault: () => {
        prevented = true;
      },
    };
    p.click(event);
    expect(prevented).toBe(true);
    // No name on the link: "download", with the ending of its type.
    expect(await next(port)).toEqual({
      type: "save_file",
      id: 2,
      name: "download.csv",
      data: btoa("a,b\n1,2\n"),
    });
    port.close();
  });

  it("saves the file of a data: address, from a link or window.open", async () => {
    const p = filePage();
    const port = connect(p);
    new p.Anchor({ href: "data:text/plain;base64,aGVsbG8=", download: "note" }).click();
    expect(await next(port)).toEqual({
      type: "save_file",
      id: 1,
      name: "note.txt",
      data: "aGVsbG8=",
    });
    // Not base64: the text as the address spells it, percent signs undone.
    expect(p.open("data:text/csv;charset=utf-8,name%2Ctotal%0AAcme%2C%E2%82%AC5")).toBeNull();
    expect(await next(port)).toEqual({
      type: "save_file",
      id: 2,
      name: "download.csv",
      data: Buffer.from("name,total\nAcme,€5").toString("base64"),
    });
    expect(p.opened).toEqual([]);
    port.close();
  });

  it("leaves alone what is no file of the page's: other links, and what the page saved itself", async () => {
    const p = filePage();
    const port = connect(p);
    // Another address: the browser's own click and window.open, as before.
    const link = new p.Anchor({ href: "https://example.com/a.pdf", download: "a.pdf" });
    link.click();
    expect(link.clicked).toBe(1);
    expect(p.open("https://example.com")).toBe("a window");
    expect(p.opened).toEqual(["https://example.com"]);
    // A blob: address this page did not make, or has revoked.
    const url = p.urls.createObjectURL(new Blob(["x"]));
    p.urls.revokeObjectURL(url);
    const gone = new p.Anchor({ href: url, download: "x.txt" });
    gone.click();
    expect(gone.clicked).toBe(1);
    // A click the page's own handler took (it called saveFile itself) is not saved twice.
    let prevented = false;
    p.click({
      defaultPrevented: true,
      target: new p.Anchor({ href: "data:text/plain,hi", download: "hi.txt" }),
      preventDefault: () => {
        prevented = true;
      },
    });
    expect(prevented).toBe(false);
    // Nothing went to Dots: the next message is this call.
    const asked = p.dots.call("ping");
    expect(await next(port)).toEqual({ type: "call", id: 1, tool: "ping", input: {} });
    port.postMessage({ type: "result", id: 1, ok: true, result: null });
    await asked;
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
