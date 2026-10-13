import { beforeEach, describe, expect, it, vi } from "vitest";

import fixture from "./__fixtures__/contract.json";

// A plain stub, not vi.fn: the mock's result tracking can report a rejection it saw as
// unhandled even though the code under test caught it.
type Reply = (cmd: string, args: unknown) => Promise<unknown>;
let reply: Reply;
const calls: [string, unknown][] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args: unknown) => {
    calls.push([cmd, args]);
    return reply(cmd, args);
  },
}));

const { commands, CommandError, toCommandError } = await import("./commands");

async function failure(p: Promise<unknown>) {
  const err = await p.catch((e: unknown) => e);
  expect(err).toBeInstanceOf(CommandError);
  const { kind, message } = err as InstanceType<typeof CommandError>;
  return { kind, message };
}

describe("commands", () => {
  beforeEach(() => {
    calls.length = 0;
    reply = async () => null;
  });

  it("passes camelCase arguments to the snake_case command", async () => {
    reply = async () => fixture.Run;
    await commands.chat("d1", "hello", null);
    expect(calls).toEqual([["chat", { dotId: "d1", message: "hello", parentRunId: null }]]);
  });

  it("returns the parsed answer", async () => {
    reply = async () => fixture.Status;
    expect(await commands.status()).toEqual(fixture.Status);
  });

  it("accepts no answer from a command without a result", async () => {
    reply = async () => undefined;
    expect(await commands.deleteDot("d1")).toBeNull();
  });

  it("refuses an answer in a shape it does not know", async () => {
    reply = async () => ({ ...fixture.Status, port: "47321" });
    const { kind, message } = await failure(commands.status());
    expect(kind).toBe("internal");
    expect(message).toMatch(/^status: unexpected port: /);

    reply = async () => ({ ...fixture.Run, extra: 1 });
    expect((await failure(commands.getRun("r1"))).kind).toBe("internal");
  });

  it("turns a Rust CmdError into a CommandError with its kind", async () => {
    // Tauri rejects with the serialized CmdError, a plain object.
    reply = async () => {
      throw { kind: "conflict", message: "run r1 is already succeeded" };
    };
    expect(await failure(commands.cancelRun("r1"))).toEqual({
      kind: "conflict",
      message: "run r1 is already succeeded",
    });
    reply = async () => {
      throw { kind: "cancelled", message: "the Claude program was not changed" };
    };
    expect((await failure(commands.updateSettings(fixture.Settings))).kind).toBe("cancelled");
  });

  it("treats anything else as an internal error and keeps its message", () => {
    for (const [input, message] of [
      ["IPC broke", "IPC broke"],
      [new Error("x"), "x"],
      [{ kind: "from-a-newer-core", message: "still readable" }, "still readable"],
      [{ code: 7 }, "[object Object]"],
    ] as const) {
      const e = toCommandError(input);
      expect({ kind: e.kind, message: e.message }).toEqual({ kind: "internal", message });
    }
  });
});
