import { describe, expect, it, vi } from "vitest";

type Handler = (e: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, h: Handler) => {
    handlers.set(name, h);
    return () => handlers.delete(name);
  },
}));

const { onLagged, onNavigate, onRuntimeEvent, onStatus } = await import("./events");

describe("events", () => {
  it("hands each listener its event's payload", async () => {
    const seen: unknown[] = [];
    await onRuntimeEvent((e) => seen.push(e));
    await onLagged(() => seen.push("lagged"));
    await onNavigate((p) => seen.push(p));
    await onStatus((s) => seen.push(s.port));
    handlers.get("runtime")!({ payload: { type: "run_updated", run: { id: "r1" } } });
    handlers.get("runtime-lagged")!({ payload: null });
    handlers.get("navigate")!({ payload: "/inbox" });
    handlers.get("status")!({ payload: { port: 47321 } });
    expect(seen).toEqual([{ type: "run_updated", run: { id: "r1" } }, "lagged", "/inbox", 47321]);
  });
});
