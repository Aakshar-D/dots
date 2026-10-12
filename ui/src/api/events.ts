import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { RuntimeEvent } from "./types";

/// Every runtime event, as the Rust side emits it (apps/dots-app/src/events.rs).
export function onRuntimeEvent(handler: (event: RuntimeEvent) => void): Promise<UnlistenFn> {
  return listen<RuntimeEvent>("runtime", (e) => handler(e.payload));
}

/// Events were dropped; refetch everything.
export function onLagged(handler: () => void): Promise<UnlistenFn> {
  return listen("runtime-lagged", () => handler());
}
