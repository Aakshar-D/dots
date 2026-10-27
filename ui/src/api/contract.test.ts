import { describe, expect, it } from "vitest";

import fixture from "./__fixtures__/contract.json";
import * as T from "./types";

// Written by apps/dots-app/src/contract.rs. Regenerate with
// `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
type SampleName = Exclude<keyof typeof fixture, "Enums" | "Limits" | "Commands">;
const schemas: Record<SampleName, { parse: (v: unknown) => unknown }> = {
  DotSpec: T.DotSpec,
  Schedule: T.Schedule.array(),
  NewDotSpec: T.DotSpec,
  Dot: T.Dot,
  Run: T.Run,
  RunEventRecord: T.RunEventRecord,
  RunEventData: T.RunEventData.array(),
  Approval: T.Approval,
  RuleCandidate: T.RuleCandidate.array(),
  RuntimeEvent: T.RuntimeEvent.array(),
  Status: T.Status,
  Settings: T.Settings,
  ChatGptStatus: T.ChatGptStatus.array(),
  SettingsSaved: T.SettingsSaved,
  DotOverview: T.DotOverview,
  Inbox: T.Inbox,
  LibraryConfig: T.LibraryConfig.array(),
  McpServerConfig: T.McpServerConfig.array(),
  LibraryItem: T.LibraryItem.array(),
  LibraryEntry: T.LibraryEntry.array(),
  ContextDocuments: T.ContextDocuments,
  McpCatalog: T.McpCatalog,
  McpProbe: T.McpProbe,
  ScannedSkill: T.ScannedSkill.array(),
  DraftRequest: T.DraftRequest.array(),
  DraftSummary: T.DraftSummary,
  InstalledPlugin: T.InstalledPlugin.array(),
  EnvVariable: T.EnvVariable.array(),
  EnvEntry: T.EnvEntry.array(),
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
  StartupError: T.StartupError,
  UpdateNote: T.UpdateNote.array(),
  Message: T.Message.array(),
  FollowUp: T.FollowUp.array(),
  Memory: T.Memory.array(),
  Watcher: T.Watcher.array(),
  BrowserStatus: T.BrowserStatus,
  AppRow: T.AppRow.array(),
  AppFile: T.AppFile.array(),
  AppCall: T.AppCall.array(),
  AppProposal: T.AppProposal.array(),
  ToolsChanged: T.ToolsChanged.array(),
  OrgSummary: T.OrgSummary.array(),
  JoinPreview: T.JoinPreview.array(),
  OrgEntry: T.OrgEntry.array(),
  SourceCreds: T.SourceCreds.array(),
  GrantStatus: T.GrantStatus.array(),
  LimitsState: T.LimitsState.array(),
  UsageSummary: T.UsageSummary,
};

// The UI parses live data with these schemas, so it must know every variant the core has.
const enums: Record<keyof typeof fixture.Enums, { options: readonly string[] }> = {
  EngineKind: T.EngineKind,
  WorkspaceMode: T.WorkspaceMode,
  TriggerKind: T.TriggerKind,
  RunStatus: T.RunStatus,
  ApprovalStatus: T.ApprovalStatus,
  ApplyState: T.ApplyState,
  Action: T.Action,
  Preset: T.Preset,
  FileStatus: T.FileStatus,
  ScheduleStatus: T.ScheduleStatus,
  CmdErrorKind: T.CmdErrorKind,
  LibraryKind: T.LibraryKind,
  CatalogStatus: T.CatalogStatus,
  MessageKind: T.MessageKind,
  FollowUpStatus: T.FollowUpStatus,
  EntryKind: T.EntryKind,
  MemorySource: T.MemorySource,
  ProposalStatus: T.ProposalStatus,
  WatcherKind: T.WatcherKind,
  PausedReason: T.PausedReason,
  UsagePeriod: T.UsagePeriod,
  FolderEvent: T.FolderEvent,
  GithubEvent: T.GithubEvent,
  GrantState: T.GrantState,
  ValueSource: T.ValueSource,
  PersonalMode: T.PersonalMode,
};

describe("Rust/TypeScript contract", () => {
  for (const [name, schema] of Object.entries(schemas)) {
    it(`parses the Rust sample of ${name}`, () => {
      expect(() => schema.parse(fixture[name as SampleName])).not.toThrow();
    });
  }

  it("rejects a field the schema does not know", () => {
    expect(() => T.Run.parse({ ...fixture.Run, extra: 1 })).toThrow();
  });

  for (const [name, schema] of Object.entries(enums)) {
    it(`knows every ${name} the core has`, () => {
      const core = fixture.Enums[name as keyof typeof fixture.Enums];
      expect([...schema.options].sort()).toEqual([...core].sort());
    });
  }

  it("has a sample of every engine event kind", () => {
    const kinds = T.RunEventData.options.map((o) => o.shape.kind.value);
    expect(fixture.RunEventData.map((e) => e.kind).sort()).toEqual([...kinds].sort());
  });

  it("has a sample of every runtime event type", () => {
    const types = T.RuntimeEvent.options.map((o) => o.shape.type.value);
    expect([...new Set(fixture.RuntimeEvent.map((e) => e.type))].sort()).toEqual([...types].sort());
  });

  it("has a sample of every Library kind and server transport", () => {
    const kinds = T.LibraryConfig.options.map((o) => o.shape.kind.value);
    expect([...new Set(fixture.LibraryConfig.map((c) => c.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    expect([...new Set(fixture.LibraryEntry.map((e) => e.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    const transports = T.McpServerConfig.options.map((o) => o.shape.transport.value);
    expect(fixture.McpServerConfig.map((c) => c.transport).sort()).toEqual(
      [...transports].sort(),
    );
  });

  it("has a catalog sample of every status", () => {
    const statuses = fixture.LibraryEntry.map((e) => e.catalog?.status);
    expect(statuses).toEqual(expect.arrayContaining(["accepted", "changed"]));
  });

  it("has a sample of every schedule kind and bound", () => {
    const kinds = T.ScheduleRule.options.map((o) => o.shape.kind.value);
    expect([...new Set(fixture.Schedule.map((s) => s.rule.kind))].sort()).toEqual(
      [...kinds].sort(),
    );
    expect(fixture.Schedule.map((s) => s.until && Object.keys(s.until)[0])).toEqual(
      expect.arrayContaining([null, "count", "date"]),
    );
  });

  it("has an org sample of every status, and sign-in details of every kind", () => {
    const statuses = T.OrgSummary.options.map((o) => o.shape.status.value);
    expect([...new Set(fixture.OrgSummary.map((o) => o.status))].sort()).toEqual(
      [...statuses].sort(),
    );
    const kinds = T.SourceCreds.options.map((o) => o.shape.kind.value);
    expect(fixture.SourceCreds.map((c) => c.kind).sort()).toEqual([...kinds].sort());
    // `use` globs with a deny, and none while no policy is accepted.
    const rules = fixture.OrgSummary.map((o) => o.use_rules);
    expect(rules).toEqual(expect.arrayContaining([null]));
    expect(rules.some((r) => r !== null && r.deny.length > 0)).toBe(true);
  });

  it("has a catalog sample in an org's scope and a changed list of each scope", () => {
    const scopes = fixture.LibraryEntry.flatMap((e) => e.catalogs.map((c) => c.scope));
    expect(scopes).toEqual(expect.arrayContaining(["personal", "acme-eng"]));
    expect(fixture.ToolsChanged.map((t) => t.scope)).toEqual(["personal", "acme-eng"]);
  });

  it("has a grant status of each source and of the reasons the editor shows", () => {
    const statuses = fixture.GrantStatus.map((g) => g.status);
    expect(statuses).toEqual(
      expect.arrayContaining([
        "ok",
        "denied_by_policy",
        "shell_secrets_forbidden",
        "org_removed",
      ]),
    );
    const sources = fixture.GrantStatus.filter((g) => g.status === "ok").map((g) => g.source);
    expect(sources).toEqual(expect.arrayContaining(["org", "personal"]));
  });

  it("has a sample of every update state", () => {
    const states = T.UpdateNote.options.map((o) => o.shape.state.value);
    expect(fixture.UpdateNote.map((n) => n.state).sort()).toEqual([...states].sort());
  });

  it("mirrors the core's limits", () => {
    expect(T.LIMITS).toEqual(fixture.Limits);
  });
});
