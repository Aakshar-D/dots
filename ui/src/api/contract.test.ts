import { describe, expect, it } from "vitest";

import fixture from "./__fixtures__/contract.json";
import * as T from "./types";

// Written by apps/dots-app/src/contract.rs. Regenerate with
// `UPDATE_CONTRACT=1 cargo test -p dots-app contract`.
const schemas: Record<keyof typeof fixture, { parse: (v: unknown) => unknown }> = {
  DotSpec: T.DotSpec,
  Dot: T.Dot,
  Run: T.Run,
  RunEventRecord: T.RunEventRecord,
  Approval: T.Approval,
  RuntimeEvent: T.RuntimeEvent.array(),
  Status: T.Status,
  Settings: T.Settings,
  SettingsSaved: T.SettingsSaved,
  DotOverview: T.DotOverview,
  Inbox: T.Inbox,
  WorktreeDiff: T.WorktreeDiff,
  CmdError: T.CmdError,
  StartupError: T.StartupError,
};

describe("Rust/TypeScript contract", () => {
  for (const [name, schema] of Object.entries(schemas)) {
    it(`parses the Rust sample of ${name}`, () => {
      expect(() => schema.parse(fixture[name as keyof typeof fixture])).not.toThrow();
    });
  }

  it("rejects a field the schema does not know", () => {
    expect(() => T.Run.parse({ ...fixture.Run, extra: 1 })).toThrow();
  });
});
