import { afterEach, describe, expect, it, vi } from "vitest";

import fixture from "./__fixtures__/contract.json";

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
  afterEach(() => vi.restoreAllMocks());

  it("hands each listener its event's payload", async () => {
    const seen: unknown[] = [];
    await onRuntimeEvent((e) => seen.push(e.type));
    await onLagged(() => seen.push("lagged"));
    await onNavigate((p) => seen.push(p));
    await onStatus((s) => seen.push(s.port));
    handlers.get("runtime")!({ payload: fixture.RuntimeEvent[0] });
    handlers.get("runtime-lagged")!({ payload: null });
    handlers.get("navigate")!({ payload: "/inbox" });
    handlers.get("status")!({ payload: fixture.Status });
    expect(seen).toEqual(["run_updated", "lagged", "/inbox", 47321]);
  });

  it("asks for a refetch when a runtime event is in a shape it does not know", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const seen: unknown[] = [];
    await onRuntimeEvent(
      (e) => seen.push(e.type),
      () => seen.push("refetch"),
    );
    await onStatus((s) => seen.push(s.port));
    handlers.get("runtime")!({ payload: { type: "from_a_newer_core" } });
    handlers.get("status")!({ payload: { port: 47321 } });
    expect(seen).toEqual(["refetch"]);
    expect(logged).toHaveBeenCalledTimes(2);
  });
});
