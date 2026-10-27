// The host side of an app's screen (app library spec section 5): the address its frame loads,
// and the bridge through which the page's `dots.call` reaches Dots. The page runs sandboxed
// (`allow-scripts` only, so in an opaque origin) and gets one MessagePort. Every message on it
// is checked here before anything is called: a call names a tool the app lists, and Dots calls
// it as the person (the page cannot say who calls). The page's client is
// apps/dots-app/src/app_client.js.
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

export type Bridge = {
  /// Sends the theme's colours, for `--dots-<token>` in the page.
  theme: (mode: ThemeName) => void;
  /// Tells the page that the app's data changed (`dots.onChange`).
  changed: () => void;
  close: () => void;
};

/// Hands the framed page `frame` its port and answers its calls: `tools` are those it may call
/// (read at each call), `call` performs one as the person.
export function connectFrame(
  frame: Window,
  options: {
    tools: readonly string[];
    call: (tool: string, input: unknown) => Promise<unknown>;
  },
): Bridge {
  const channel = new MessageChannel();
  const port = channel.port1;
  let open = true;
  const send = (message: unknown) => {
    if (open) port.postMessage(message);
  };
  port.onmessage = (event: MessageEvent) => {
    const parsed = Call.safeParse(event.data);
    if (!parsed.success) return;
    const { id, tool, input } = parsed.data;
    if (!options.tools.includes(tool)) {
      send({ type: "result", id, ok: false, error: `this app has no tool ${tool}` });
      return;
    }
    // A call that throws at once, or answers without a promise, is answered like any failure.
    Promise.resolve()
      .then(() => options.call(tool, input ?? {}))
      .then(
        (result) => send({ type: "result", id, ok: true, result }),
        (e: unknown) => send({ type: "result", id, ok: false, error: errorMessage(e) }),
      );
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
