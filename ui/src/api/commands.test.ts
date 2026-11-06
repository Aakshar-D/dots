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
    expect(await commands.decideRunApprovals("r1", ["a1", "a2"], true, null)).toEqual([fixture.Approval]);
    expect(calls).toEqual([
      ["decide_approval", { id: "a1", approved: true, note: null, allowRule: "Bash(git push)" }],
      ["decide_approval", { id: "a1", approved: false, note: "no", allowRule: null }],
      ["approval_rule_candidates", { id: "a1" }],
      ["decide_run_approvals", { runId: "r1", ids: ["a1", "a2"], approved: true, note: null }],
    ]);
  });

  it("scans and imports Claude Code agents as the core names them", async () => {
    reply = async () => fixture.ScannedAgent;
    expect(await commands.scanClaudeAgents()).toEqual(fixture.ScannedAgent);
    reply = async () => fixture.AgentImport;
    expect(await commands.importClaudeAgents(["C:/a.md"], "acme", "platform")).toEqual(
      fixture.AgentImport,
    );
    await commands.importClaudeAgents(["C:/b.md"]);
    expect(calls).toEqual([
      ["scan_claude_agents", undefined],
      ["import_claude_agents", { paths: ["C:/a.md"], orgId: "acme", workspaceId: "platform" }],
      ["import_claude_agents", { paths: ["C:/b.md"], orgId: null, workspaceId: null }],
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
    reply = async () => [fixture.ScheduleSkip];
    expect(await commands.scheduleSkips("d1")).toEqual([fixture.ScheduleSkip]);
    reply = async () => [fixture.TopicInfo];
    expect(await commands.listTopics("d1")).toEqual([fixture.TopicInfo]);
    reply = async () => fixture.TopicInfo;
    await commands.renameTopic("r1", "Trip");
    await commands.setTopicArchived("r1", true);
    reply = async () => "r2";
    expect(await commands.moveRun("r3", "r2")).toBe("r2");
    reply = async () => null;
    await commands.mergeTopics("r1", "r2");
    reply = async () => fixture.FollowUp[0];
    await commands.followUpCancel("f1");
    expect(calls).toEqual([
      ["messages_list", { dotId: null }],
      ["messages_list", { dotId: "d1" }],
      ["message_reply", { id: "m1", text: "yes" }],
      ["message_dismiss", { id: "m1" }],
      ["message_mark_read", { id: "m1" }],
      ["follow_ups_list", { dotId: null }],
      ["schedule_skips", { dotId: "d1", limit: undefined }],
      ["list_topics", { dotId: "d1" }],
      ["rename_topic", { rootRunId: "r1", title: "Trip" }],
      ["set_topic_archived", { rootRunId: "r1", archived: true }],
      ["move_run", { runId: "r3", toRootRunId: "r2" }],
      ["merge_topics", { fromRootRunId: "r1", intoRootRunId: "r2" }],
      ["follow_up_cancel", { id: "f1" }],
    ]);
  });

  it("sends the browser commands as the core names them", async () => {
    reply = async () => null;
    expect(await commands.browserOpen("d1")).toBeNull();
    reply = async () => fixture.BrowserStatus;
    expect(await commands.browserStatus()).toEqual(fixture.BrowserStatus);
    reply = async () => "data:image/png;base64,iVBORw==";
    expect(await commands.browserScreenshot("r1", "page-1.png")).toBe(
      "data:image/png;base64,iVBORw==",
    );
    expect(calls).toEqual([
      ["browser_open", { dotId: "d1" }],
      ["browser_status", undefined],
      ["browser_screenshot", { runId: "r1", name: "page-1.png" }],
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
    reply = async () => null;
    expect(await commands.confirmOrigin("i1")).toBeNull();
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
      // No scope: Personal.
      ["library_check", { id: "i1", scope: undefined }],
      ["library_accept", { id: "i1", fingerprint: "fp2", scope: undefined }],
      ["confirm_origin", { serverId: "i1", scope: undefined }],
      ["library_scan_skills", { path: "C:/skills" }],
      ["context_documents", { id: "i1" }],
      ["installed_claude_plugins", undefined],
      ["environment_variables", { names: ["GITHUB_TOKEN"] }],
    ]);
  });

  it("checks, accepts and confirms in an org's scope as the core names them", async () => {
    reply = async () => fixture.McpCatalog;
    await commands.libraryCheck("i1", "acme-eng");
    await commands.libraryAccept("i1", "fp2", "acme-eng");
    reply = async () => null;
    await commands.confirmOrigin("i1", "acme-eng");
    expect(calls).toEqual([
      ["library_check", { id: "i1", scope: "acme-eng" }],
      ["library_accept", { id: "i1", fingerprint: "fp2", scope: "acme-eng" }],
      ["confirm_origin", { serverId: "i1", scope: "acme-eng" }],
    ]);
    // A declined dialog is a rejection the screen can tell apart.
    reply = async () => {
      throw { kind: "cancelled", message: "declined: nothing was sent or saved" };
    };
    expect((await failure(commands.libraryCheck("i1", "acme-eng"))).kind).toBe("cancelled");
  });

  it("sends the org commands as the core names them, never taking sign-in details", async () => {
    const [token, , , age] = fixture.SourceCreds.map((c) => T.SourceCreds.parse(c));
    reply = async () => fixture.OrgSummary;
    expect(await commands.orgList()).toEqual(fixture.OrgSummary);
    reply = async () => fixture.JoinPreview[1];
    expect(await commands.orgPreview("code")).toEqual(fixture.JoinPreview[1]);
    reply = async () => "age1sample";
    expect(await commands.orgSopsKey("design")).toBe("age1sample");
    reply = async () => fixture.OrgSummary[0];
    await commands.orgJoin("code", token!);
    await commands.orgJoin("sops-code", null);
    await commands.orgUpdate("acme-eng", "code2");
    await commands.orgUpdate("design", "code3", age!);
    await commands.orgRefresh("acme-eng");
    reply = async () => null;
    expect(await commands.orgRemove("acme-eng")).toBeNull();
    reply = async () => fixture.OrgEntry;
    expect(await commands.orgEntries("acme-eng")).toEqual(fixture.OrgEntry);
    expect(calls).toEqual([
      ["org_list", undefined],
      ["org_preview", { code: "code" }],
      ["org_sops_key", { orgId: "design" }],
      ["org_join", { code: "code", creds: token }],
      ["org_join", { code: "sops-code", creds: null }],
      ["org_update", { orgId: "acme-eng", code: "code2", creds: null }],
      ["org_update", { orgId: "design", code: "code3", creds: age }],
      ["org_refresh", { orgId: "acme-eng" }],
      ["org_remove", { orgId: "acme-eng" }],
      ["org_entries", { orgId: "acme-eng" }],
    ]);
    // An answer holding a value or the sign-in details is a shape this UI refuses.
    reply = async () => [{ ...fixture.OrgEntry[1], secret: "x" }];
    expect((await failure(commands.orgEntries("acme-eng"))).kind).toBe("internal");
    reply = async () => ({ ...fixture.OrgSummary[0], creds: token });
    expect((await failure(commands.orgRefresh("acme-eng"))).kind).toBe("internal");
  });

  it("asks what the person may not do with a dot", async () => {
    reply = async () => fixture.DotAccess[0];
    expect(await commands.dotAccess("D1")).toEqual(fixture.DotAccess[0]);
    expect(calls).toEqual([["dot_access", { dotId: "D1" }]]);
    reply = async () => ({ ...fixture.DotAccess[1], view: null });
    expect((await failure(commands.dotAccess("D1"))).kind).toBe("internal");
  });

  it("sends the account commands as the core names them", async () => {
    reply = async () => fixture.Identity;
    expect(await commands.identityGet()).toEqual(fixture.Identity);
    await commands.profileSet("Asha Rao", "asha@acme.com");
    reply = async () => fixture.CurrentScope;
    expect(await commands.scopeGet()).toEqual(fixture.CurrentScope);
    await commands.scopeSet("personal", "default");
    expect(calls).toEqual([
      ["identity_get", undefined],
      ["profile_set", { name: "Asha Rao", email: "asha@acme.com" }],
      ["scope_get", undefined],
      ["scope_set", { account: "personal", workspace: "default" }],
    ]);
    reply = async () => ({ ...fixture.Identity, secret: "x" });
    expect((await failure(commands.identityGet())).kind).toBe("internal");
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

  it("sends the vault's one-time offers as the core names them", async () => {
    reply = async () => ["GITHUB_TOKEN"];
    expect(await commands.envImportCandidates()).toEqual(["GITHUB_TOKEN"]);
    expect(await commands.envImport(["GITHUB_TOKEN"])).toEqual(["GITHUB_TOKEN"]);
    reply = async () => ["dots-0.1.0-2026-10-20-091700.db"];
    expect(await commands.envOldBackups()).toEqual(["dots-0.1.0-2026-10-20-091700.db"]);
    reply = async () => null;
    await commands.envImportDismiss();
    // The backups that could not be deleted, by file name.
    reply = async () => ["dots-0.0.9-stuck.db"];
    expect(await commands.envOldBackupsAnswer(true)).toEqual(["dots-0.0.9-stuck.db"]);
    expect(calls.slice(-5)).toEqual([
      ["env_import_candidates", undefined],
      ["env_import", { names: ["GITHUB_TOKEN"] }],
      ["env_old_backups", undefined],
      ["env_import_dismiss", undefined],
      ["env_old_backups_answer", { delete: true }],
    ]);
  });

  it("refuses a Library answer in a shape it does not know", async () => {
    reply = async () => [{ ...fixture.LibraryEntry[0], extra: 1 }];
    expect((await failure(commands.libraryList())).kind).toBe("internal");
    reply = async () => fixture.EnvVariable.map((v) => ({ ...v, value: "secret" }));
    expect((await failure(commands.environmentVariables(["A"]))).kind).toBe("internal");
  });

  it("sends the team, project and workflow commands as the core names them", async () => {
    const team = T.TeamSpec.parse(fixture.TeamSpec);
    const project = T.ProjectSpec.parse(fixture.ProjectSpec[0]);
    const spec = T.WorkflowSpec.parse(fixture.WorkflowSpec[0]);
    reply = async () => fixture.Team;
    expect(await commands.teamsList()).toEqual(fixture.Team);
    reply = async () => fixture.Team[0];
    await commands.teamGet("t1");
    await commands.teamCreate(team);
    await commands.teamUpdate("t1", team);
    reply = async () => null;
    expect(await commands.teamDelete("t1")).toBeNull();
    reply = async () => fixture.Project;
    await commands.projectAdd("t1", project);
    await commands.projectUpdate("p1", project);
    reply = async () => null;
    await commands.projectDelete("p1");
    await commands.openProjectFolder("p1");
    await commands.openBranchFolder("w1", "dots/dev/01J0RUN");
    reply = async () => fixture.Workflow;
    expect(await commands.workflowsList("t1")).toEqual(fixture.Workflow);
    reply = async () => fixture.Workflow[0];
    await commands.workflowGet("f1");
    await commands.workflowCreate("t1", spec);
    await commands.workflowUpdate("f1", spec);
    reply = async () => null;
    await commands.workflowDelete("f1");
    expect(await commands.workflowCheckSpec("t1", spec)).toBeNull();
    reply = async () => fixture.WorkflowProblem[0];
    expect(await commands.workflowCheckSpec("t1", spec)).toEqual(fixture.WorkflowProblem[0]);
    reply = async () => "http://127.0.0.1:47321/workflows/f1/trigger";
    expect(await commands.workflowWebhookUrl("f1")).toBe(
      "http://127.0.0.1:47321/workflows/f1/trigger",
    );
    reply = async () => "token";
    expect(await commands.regenerateWorkflowToken("f1")).toBe("token");
    expect(calls).toEqual([
      ["teams_list", undefined],
      ["team_get", { id: "t1" }],
      ["team_create", { spec: team }],
      ["team_update", { id: "t1", spec: team }],
      ["team_delete", { id: "t1" }],
      ["project_add", { teamId: "t1", spec: project }],
      ["project_update", { id: "p1", spec: project }],
      ["project_delete", { id: "p1" }],
      ["open_project_folder", { projectId: "p1", workflowRunId: null, branch: null }],
      [
        "open_project_folder",
        { projectId: null, workflowRunId: "w1", branch: "dots/dev/01J0RUN" },
      ],
      ["workflows_list", { teamId: "t1" }],
      ["workflow_get", { id: "f1" }],
      ["workflow_create", { teamId: "t1", spec }],
      ["workflow_update", { id: "f1", spec }],
      ["workflow_delete", { id: "f1" }],
      ["workflow_check_spec", { teamId: "t1", spec }],
      ["workflow_check_spec", { teamId: "t1", spec }],
      ["workflow_webhook_url", { id: "f1" }],
      ["regenerate_workflow_token", { id: "f1" }],
    ]);
  });

  it("sends the workflow run commands as the core names them, with the tips the check showed", async () => {
    const tips = T.CheckTip.array().parse(fixture.CheckTip);
    reply = async () => fixture.WorkflowRun;
    expect(await commands.workflowRunsList({ team_id: "t1", statuses: ["waiting"] })).toEqual(
      fixture.WorkflowRun,
    );
    await commands.workflowRunsList({});
    await commands.workflowRunsForOrigins(["r1", "r2"]);
    reply = async () => fixture.WorkflowRun[0];
    await commands.workflowRunGet("w1");
    reply = async () => fixture.StepDotRun;
    expect(await commands.workflowRunDotRuns("w1")).toEqual(fixture.StepDotRun);
    reply = async () => fixture.WorkflowRun[0];
    await commands.workflowRunStart("f1", "p1", "Fix the icon.", false);
    await commands.workflowRunStart("f1", "p1", "Fix the icon.", true);
    reply = async () => fixture.CheckView;
    expect(await commands.workflowCheck("w1")).toEqual(fixture.CheckView);
    reply = async () => fixture.WorktreeDiff;
    expect(await commands.workflowBranchDiff("w1", "dots/dev/01J0RUN", "abc")).toEqual(
      fixture.WorktreeDiff,
    );
    reply = async () => fixture.WorkflowRun[1];
    await commands.workflowApprove("w1", null, "s1", tips);
    await commands.workflowApprove("w1", "Looks right.", "s1", []);
    await commands.workflowSendBack("w1", "Keep the old icon.", "s1");
    await commands.workflowAddBudget("w1");
    await commands.workflowAllowSendBack("w1");
    await commands.workflowRetryStep("w1");
    await commands.workflowStop("w1");
    expect(calls).toEqual([
      ["workflow_runs_list", { filter: { team_id: "t1", statuses: ["waiting"] } }],
      ["workflow_runs_list", { filter: {} }],
      ["workflow_runs_for_origins", { runIds: ["r1", "r2"] }],
      ["workflow_run_get", { id: "w1" }],
      ["workflow_run_dot_runs", { id: "w1" }],
      [
        "workflow_run_start",
        { workflowId: "f1", projectId: "p1", task: "Fix the icon.", test: false },
      ],
      [
        "workflow_run_start",
        { workflowId: "f1", projectId: "p1", task: "Fix the icon.", test: true },
      ],
      ["workflow_check", { id: "w1" }],
      ["workflow_branch_diff", { id: "w1", branch: "dots/dev/01J0RUN", commit: "abc" }],
      ["workflow_approve", { id: "w1", note: null, stepRunId: "s1", tips }],
      ["workflow_approve", { id: "w1", note: "Looks right.", stepRunId: "s1", tips: [] }],
      ["workflow_send_back", { id: "w1", note: "Keep the old icon.", stepRunId: "s1" }],
      ["workflow_add_budget", { id: "w1" }],
      ["workflow_allow_send_back", { id: "w1" }],
      ["workflow_retry_step", { id: "w1" }],
      ["workflow_stop", { id: "w1" }],
    ]);
    // A gone branch is sent as null, the way the check showed it.
    expect(tips.map((t) => t.commit)).toContain(null);
  });

  it("refuses a workflow run in a shape it does not know", async () => {
    const [run] = fixture.WorkflowRun;
    reply = async () => ({ ...run, steps: [{ ...run.steps[0], extra: 1 }] });
    expect((await failure(commands.workflowRunGet("w1"))).kind).toBe("internal");
  });

  it("sends the connector commands as the core names them", async () => {
    reply = async () => fixture.ConnectorEntry;
    expect(await commands.connectorList()).toEqual(fixture.ConnectorEntry);
    reply = async () => fixture.ConnectorEntry[0];
    await commands.connectorConnect("github", { token: "ghp_x" }, ["d1"]);
    await commands.connectorSignIn("notion", ["d1"]);
    await commands.connectorSetDots("notion", []);
    await commands.connectorDisconnect("notion");
    reply = async () => null;
    await commands.connectorOpenPage("github", "instructions");
    await commands.connectorSignInCancel("notion");
    reply = async () => fixture.RuntimeCheck[2];
    expect((await commands.connectorRuntime("slack")).ok).toBe(false);
    expect(calls).toEqual([
      ["connector_list", undefined],
      ["connector_connect", { id: "github", values: { token: "ghp_x" }, dotIds: ["d1"] }],
      ["connector_sign_in", { id: "notion", dotIds: ["d1"] }],
      ["connector_set_dots", { id: "notion", dotIds: [] }],
      ["connector_disconnect", { id: "notion" }],
      ["connector_open_page", { id: "github", page: "instructions" }],
      ["connector_sign_in_cancel", { id: "notion" }],
      ["connector_runtime", { id: "slack" }],
    ]);
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
