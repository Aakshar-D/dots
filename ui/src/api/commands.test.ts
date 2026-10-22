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

  it("sends the check-in commands as the core names them", async () => {
    reply = async () => fixture.Message;
    expect(await commands.messagesList(null)).toEqual(fixture.Message);
    await commands.messagesList("d1");
    reply = async () => fixture.Run;
    await commands.messageReply("m1", "yes");
    reply = async () => fixture.Message[0];
    await commands.messageDismiss("m1");
    await commands.messageMarkRead("m1");
    reply = async () => fixture.FollowUp;
    expect(await commands.followUpsList(null)).toEqual(fixture.FollowUp);
    reply = async () => fixture.FollowUp[0];
    await commands.followUpCancel("f1");
    expect(calls).toEqual([
      ["messages_list", { dotId: null }],
      ["messages_list", { dotId: "d1" }],
      ["message_reply", { id: "m1", text: "yes" }],
      ["message_dismiss", { id: "m1" }],
      ["message_mark_read", { id: "m1" }],
      ["follow_ups_list", { dotId: null }],
      ["follow_up_cancel", { id: "f1" }],
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
    reply = async () => fixture.ContextDocuments;
    expect(await commands.contextDocuments("i1")).toEqual(fixture.ContextDocuments);
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
      ["context_documents", { id: "i1" }],
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

  it("drafts, checks and saves a skill with the arguments the core names", async () => {
    const [first, change] = fixture.DraftRequest;
    reply = async () => "---\nname: pr-tests\n---\nBody\n";
    expect(await commands.skillDraft("dialog-1", first!)).toContain("pr-tests");
    await commands.skillDraft("dialog-1", change!);
    reply = async () => null;
    await commands.skillDraftCancel("dialog-1");
    reply = async () => fixture.DraftSummary;
    expect(await commands.skillCheck("text")).toEqual(fixture.DraftSummary);
    reply = async () => fixture.LibraryItem[1];
    expect(await commands.skillCreate("text")).toEqual(fixture.LibraryItem[1]);
    expect(calls).toEqual([
      ["skill_draft", { dialog: "dialog-1", request: first }],
      ["skill_draft", { dialog: "dialog-1", request: change }],
      ["skill_draft_cancel", { dialog: "dialog-1" }],
      ["skill_check", { text: "text" }],
      ["skill_create", { text: "text" }],
    ]);
  });

  it("says what to change in a draft that cannot be saved", async () => {
    reply = async () => {
      throw { kind: "invalid", message: "Add a description: line between the --- lines." };
    };
    const { kind, message } = await failure(commands.skillCheck("---\nname: a\n---\nB\n"));
    expect(kind).toBe("invalid");
    expect(message).toContain("description");
  });

  it("sends the Env tab's commands as the core names them, and never takes a secret back", async () => {
    reply = async () => fixture.EnvEntry;
    expect(await commands.envList()).toEqual(fixture.EnvEntry);
    reply = async () => fixture.EnvEntry[0];
    await commands.envAdd("GITHUB_TOKEN", "secret", "ghp_x");
    await commands.envSet("GITHUB_TOKEN", "ghp_y");
    reply = async () => null;
    expect(await commands.envRemove("GITHUB_TOKEN")).toBeNull();
    expect(calls).toEqual([
      ["env_list", undefined],
      ["env_add", { name: "GITHUB_TOKEN", kind: "secret", value: "ghp_x" }],
      ["env_set", { name: "GITHUB_TOKEN", value: "ghp_y" }],
      ["env_remove", { name: "GITHUB_TOKEN" }],
    ]);
    reply = async () => [{ ...fixture.EnvEntry[0], secret: "ghp_x" }];
    expect((await failure(commands.envList())).kind).toBe("internal");
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
