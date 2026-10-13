import { beforeEach, describe, expect, it, vi } from "vitest";

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

describe("commands", () => {
  beforeEach(() => {
    calls.length = 0;
    reply = async () => null;
  });

  it("passes camelCase arguments to the snake_case command", async () => {
    await commands.chat("d1", "hello", null);
    expect(calls).toEqual([["chat", { dotId: "d1", message: "hello", parentRunId: null }]]);
  });

  it("turns a Rust CmdError into a CommandError with its kind", async () => {
    // Tauri rejects with the serialized CmdError, a plain object.
    reply = async () => {
      throw { kind: "conflict", message: "run r1 is already succeeded" };
    };
    const err = await commands.cancelRun("r1").catch((e: unknown) => e);
    expect(err).toBeInstanceOf(CommandError);
    const { kind, message } = err as InstanceType<typeof CommandError>;
    expect({ kind, message }).toEqual({ kind: "conflict", message: "run r1 is already succeeded" });
  });

  it("treats anything else as an internal error", () => {
    for (const [input, message] of [
      ["IPC broke", "IPC broke"],
      [new Error("x"), "x"],
    ] as const) {
      const e = toCommandError(input);
      expect({ kind: e.kind, message: e.message }).toEqual({ kind: "internal", message });
    }
  });
});
