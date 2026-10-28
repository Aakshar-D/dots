// The host side of an app's screen (app library spec section 5): the address its frame loads,
// and the bridge through which the page's `dots.call` reaches Dots. The page runs sandboxed
// (`allow-scripts` only, so in an opaque origin) and gets one MessagePort. Every message on it
// is checked here before anything is called: a call names a tool the app lists, and Dots calls
// it as the person (the page cannot say who calls); a run the page asks to open is one the app
// started, which the core checks. The page's client is apps/dots-app/src/app_client.js.
import { z } from "zod";

import { errorMessage } from "@/lib/notify";
import { THEMES, type ThemeName } from "@/lib/tokens";

/// Where the `dots-app` protocol serves the apps' pages (`ORIGIN` in
/// apps/dots-app/src/app_frame.rs); Dots' own CSP lets it be framed.
export const APP_ORIGIN = "http://dots-app.localhost";
/// The frame's `sandbox`: scripts, and nothing else (no same origin, forms or popups).
export const SANDBOX = "allow-scripts";

/// The page of the app `name` at `version`.
export const appSrc = (name: string, version: number) =>
  `${APP_ORIGIN}/app/${encodeURIComponent(name)}/${version}/index.html`;
/// The page of a review card's preview.
export const previewSrc = (id: string) =>
  `${APP_ORIGIN}/preview/${encodeURIComponent(id)}/index.html`;

/// A call the page asks for. Anything else on the port is ignored.
const Call = z.strictObject({
  type: z.literal("call"),
  id: z.number().int().positive(),
  tool: z.string(),
  input: z.unknown(),
});
/// The page asks to open a run in its conversation (`dots.openRun`).
const OpenRun = z.strictObject({
  type: z.literal("open_run"),
  id: z.number().int().positive(),
  run_id: z.string(),
});

export type Bridge = {
  /// Sends the theme's colours, for `--dots-<token>` in the page.
  theme: (mode: ThemeName) => void;
  /// Tells the page that the app's data changed (`dots.onChange`).
  changed: () => void;
  close: () => void;
};

/// Hands the framed page `frame` its port and answers its calls: `tools` are those it may call
/// (read at each call), `call` performs one as the person, and `openRun` opens a run the app
/// started (a page without it, a preview's, is refused).
export function connectFrame(
  frame: Window,
  options: {
    tools: readonly string[];
    call: (tool: string, input: unknown) => Promise<unknown>;
    openRun?: (runId: string) => Promise<void>;
  },
): Bridge {
  const channel = new MessageChannel();
  const port = channel.port1;
  let open = true;
  const send = (message: unknown) => {
    if (open) port.postMessage(message);
  };
  /// Answers the request `id` with what `work` gives, or its error.
  const answer = (id: number, work: () => Promise<unknown> | unknown) =>
    // Work that throws at once, or answers without a promise, is answered like any failure.
    Promise.resolve()
      .then(work)
      .then(
        (result) => send({ type: "result", id, ok: true, result }),
        (e: unknown) => send({ type: "result", id, ok: false, error: errorMessage(e) }),
      );
  port.onmessage = (event: MessageEvent) => {
    const open = OpenRun.safeParse(event.data);
    if (open.success) {
      const { id, run_id } = open.data;
      answer(id, async () => {
        if (!options.openRun) throw new Error("this screen cannot open runs");
        await options.openRun(run_id);
        return null;
      });
      return;
    }
    const parsed = Call.safeParse(event.data);
    if (!parsed.success) return;
    const { id, tool, input } = parsed.data;
    if (!options.tools.includes(tool)) {
      send({ type: "result", id, ok: false, error: `this app has no tool ${tool}` });
      return;
    }
    answer(id, () => options.call(tool, input ?? {}));
  };
  frame.postMessage({ type: "dots:connect" }, "*", [channel.port2]);
  return {
    theme: (mode) => send({ type: "theme", mode, tokens: THEMES[mode] }),
    changed: () => send({ type: "changed" }),
    close: () => {
      open = false;
      port.close();
    },
  };
}
