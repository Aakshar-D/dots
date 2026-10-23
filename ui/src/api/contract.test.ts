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
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
  StartupError: T.StartupError,
  UpdateNote: T.UpdateNote.array(),
  Message: T.Message.array(),
  FollowUp: T.FollowUp.array(),
  BrowserStatus: T.BrowserStatus,
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
    expect(fixture.RuntimeEvent.map((e) => e.type).sort()).toEqual([...types].sort());
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

  it("has a sample of every update state", () => {
    const states = T.UpdateNote.options.map((o) => o.shape.state.value);
    expect(fixture.UpdateNote.map((n) => n.state).sort()).toEqual([...states].sort());
  });

  it("mirrors the core's limits", () => {
    expect(T.LIMITS).toEqual(fixture.Limits);
  });
});
