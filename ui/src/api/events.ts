import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { RuntimeEvent, Status } from "./types";

/// Every runtime event, as the Rust side emits it (apps/dots-app/src/events.rs).
export function onRuntimeEvent(handler: (event: RuntimeEvent) => void): Promise<UnlistenFn> {
  return listen<RuntimeEvent>("runtime", (e) => handler(e.payload));
}

/// Events were dropped; refetch everything.
export function onLagged(handler: () => void): Promise<UnlistenFn> {
  return listen("runtime-lagged", () => handler());
}

/// The tray asks for a route, e.g. "/inbox".
export function onNavigate(handler: (path: string) => void): Promise<UnlistenFn> {
  return listen<string>("navigate", (e) => handler(e.payload));
}

/// The status changed outside the UI (the tray paused or resumed schedules).
export function onStatus(handler: (status: Status) => void): Promise<UnlistenFn> {
  return listen<Status>("status", (e) => handler(e.payload));
}
