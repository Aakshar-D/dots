// The events the Rust side emits (apps/dots-app/src/events.rs and tray.rs). Payloads are
// parsed with their schemas before a handler sees them.
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { z } from "zod";

import * as T from "./types";
import type { RuntimeEvent, Status } from "./types";

function parse<S extends z.ZodType>(event: string, schema: S, payload: unknown) {
  const parsed = schema.safeParse(payload);
  if (parsed.success) return parsed.data;
  console.error(`event "${event}" in a shape this UI does not know`, parsed.error.issues[0]);
  return undefined;
}

/// Every runtime event. `onInvalid` is called for an event in a shape this UI does not know:
/// the caller cannot patch its caches from it, so it should refetch, as after `onLagged`.
export function onRuntimeEvent(
  handler: (event: RuntimeEvent) => void,
  onInvalid: () => void = () => {},
): Promise<UnlistenFn> {
  return listen("runtime", (e) => {
    const event = parse("runtime", T.RuntimeEvent, e.payload);
    if (event) handler(event);
    else onInvalid();
  });
}

/// Events were dropped; refetch everything.
export function onLagged(handler: () => void): Promise<UnlistenFn> {
  return listen("runtime-lagged", () => handler());
}

/// The tray asks for a route, e.g. "/inbox".
export function onNavigate(handler: (path: string) => void): Promise<UnlistenFn> {
  return listen("navigate", (e) => {
    const path = parse("navigate", z.string(), e.payload);
    if (path !== undefined) handler(path);
  });
}

/// The status changed outside the UI (the tray paused or resumed schedules).
export function onStatus(handler: (status: Status) => void): Promise<UnlistenFn> {
  return listen("status", (e) => {
    const status = parse("status", T.Status, e.payload);
    if (status) handler(status);
  });
}
