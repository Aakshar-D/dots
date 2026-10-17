import { beforeEach, describe, expect, it, vi } from "vitest";

import fixture from "./__fixtures__/contract.json";
import * as T from "./types";

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

  it("sends an approval's rule and a run's batch as the core names them", async () => {
    reply = async () => fixture.Approval;
    await commands.decideApproval("a1", true, null, "Bash(git push)");
    await commands.decideApproval("a1", false, "no", null);
    reply = async () => fixture.RuleCandidate;
    expect(await commands.approvalRuleCandidates("a1")).toEqual(fixture.RuleCandidate);
    reply = async () => [fixture.Approval];
    expect(await commands.decideRunApprovals("r1", true, null)).toEqual([fixture.Approval]);
    expect(calls).toEqual([
      ["decide_approval", { id: "a1", approved: true, note: null, allowRule: "Bash(git push)" }],
      ["decide_approval", { id: "a1", approved: false, note: "no", allowRule: null }],
      ["approval_rule_candidates", { id: "a1" }],
      ["decide_run_approvals", { runId: "r1", approved: true, note: null }],
    ]);
  });

  it("dismisses a schedule's attention by its dot", async () => {
    reply = async () => null;
    expect(await commands.dismissScheduleAttention("d1")).toBeNull();
    expect(calls).toEqual([["dismiss_schedule_attention", { dotId: "d1" }]]);
  });

  it("sends the Library's arguments as the core names them", async () => {
    const config = T.LibraryConfig.parse(fixture.LibraryConfig[0]);
    const server = T.McpServerConfig.parse(fixture.McpServerConfig[0]);
    reply = async () => fixture.LibraryEntry;
    expect(await commands.libraryList()).toEqual(fixture.LibraryEntry);
    reply = async () => fixture.LibraryItem[0];
    await commands.libraryAdd("github", config, "fp1");
    await commands.libraryAdd("github", config, null);
    await commands.libraryUpdate("i1", config);
    reply = async () => null;
    expect(await commands.libraryRemove("i1", true)).toBeNull();
    reply = async () => fixture.McpProbe;
    expect(await commands.libraryProbe(server)).toEqual(fixture.McpProbe);
    reply = async () => fixture.McpCatalog;
    expect(await commands.libraryCheck("i1")).toEqual(fixture.McpCatalog);
    expect(await commands.libraryAccept("i1", "fp2")).toEqual(fixture.McpCatalog);
    reply = async () => fixture.ScannedSkill;
    expect(await commands.libraryScanSkills("C:/skills")).toEqual(fixture.ScannedSkill);
    reply = async () => fixture.InstalledPlugin;
    expect(await commands.installedClaudePlugins()).toEqual(fixture.InstalledPlugin);
    reply = async () => fixture.EnvVariable;
    expect(await commands.environmentVariables(["GITHUB_TOKEN"])).toEqual(fixture.EnvVariable);
    expect(calls).toEqual([
      ["library_list", undefined],
      ["library_add", { name: "github", config, accept: "fp1" }],
      ["library_add", { name: "github", config, accept: null }],
      ["library_update", { id: "i1", config }],
      ["library_remove", { id: "i1", confirm: true }],
      ["library_probe", { config: server }],
      ["library_check", { id: "i1" }],
      ["library_accept", { id: "i1", fingerprint: "fp2" }],
      ["library_scan_skills", { path: "C:/skills" }],
      ["installed_claude_plugins", undefined],
      ["environment_variables", { names: ["GITHUB_TOKEN"] }],
    ]);
  });

  it("refuses to remove a used Library item without a yes, with the dots named", async () => {
    reply = async () => {
      throw {
        kind: "conflict",
        message: 'the skill "pdf" is used by audit; removing it takes it out of them',
      };
    };
    const { kind, message } = await failure(commands.libraryRemove("i1", false));
    expect(kind).toBe("conflict");
    expect(message).toContain("audit");
  });

  it("refuses a Library answer in a shape it does not know", async () => {
    reply = async () => [{ ...fixture.LibraryEntry[0], extra: 1 }];
    expect((await failure(commands.libraryList())).kind).toBe("internal");
    reply = async () => fixture.EnvVariable.map((v) => ({ ...v, value: "secret" }));
    expect((await failure(commands.environmentVariables(["A"]))).kind).toBe("internal");
  });

  it("invokes exactly the commands the app registers", async () => {
    // A wrapper calls `invoke` before it first waits, so calling each one without arguments
    // lists the names with no backend. Their answers, mostly rejections here, do not matter.
    const wrappers: Record<string, (...args: never[]) => Promise<unknown>> = commands;
    await Promise.allSettled(Object.values(wrappers).map((wrapper) => wrapper()));
    const invoked = [...new Set(calls.map(([cmd]) => cmd))];
    expect(invoked.sort()).toEqual([...fixture.Commands].sort());
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
