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

const { onLagged, onNavigate, onRuntimeEvent, onStatus, onUpdate } = await import("./events");

describe("events", () => {
  afterEach(() => vi.restoreAllMocks());

  it("hands each listener its event's payload", async () => {
    const seen: unknown[] = [];
    await onRuntimeEvent(
      (e) => seen.push(e.type),
      () => seen.push("refetch"),
    );
    await onLagged(() => seen.push("lagged"));
    await onNavigate((p) => seen.push(p));
    await onStatus((s) => seen.push(s.port));
    await onUpdate((n) => seen.push(n.state));
    handlers.get("runtime")!({ payload: fixture.RuntimeEvent[0] });
    handlers.get("runtime-lagged")!({ payload: null });
    handlers.get("navigate")!({ payload: "/inbox" });
    handlers.get("status")!({ payload: fixture.Status });
    handlers.get("update")!({ payload: fixture.UpdateNote[1] });
    expect(seen).toEqual(["run_updated", "lagged", "/inbox", 47321, "ready"]);
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
    // The log names the type that arrived, when the payload has one.
    expect(logged.mock.calls.map(([message]) => message)).toEqual([
      'event "runtime" of type "from_a_newer_core" in a shape this UI does not know',
      'event "status" in a shape this UI does not know',
    ]);
  });

  it("logs everything that is wrong with a known event, and a type only when it is text", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    const refetches: string[] = [];
    await onRuntimeEvent(
      () => {},
      () => refetches.push("refetch"),
    );
    const { run } = fixture.RuntimeEvent[0] as { run: Record<string, unknown> };
    handlers.get("runtime")!({ payload: { type: "run_updated", run: { ...run, id: 7, status: "?" } } });
    handlers.get("runtime")!({ payload: { type: 7 } });
    handlers.get("runtime")!({ payload: null });

    expect(refetches).toHaveLength(3);
    const [known, numbered, empty] = logged.mock.calls;
    expect(known[0]).toBe('event "runtime" of type "run_updated" in a shape this UI does not know');
    expect((known[1] as { path: unknown[] }[]).map((issue) => issue.path)).toEqual([
      ["run", "id"],
      ["run", "status"],
    ]);
    expect(numbered[0]).toBe('event "runtime" in a shape this UI does not know');
    expect(empty[0]).toBe('event "runtime" in a shape this UI does not know');
  });
});
