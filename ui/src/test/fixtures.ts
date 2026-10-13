// Test data: the Rust-written contract samples, parsed with the UI's own schemas.
import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";

import { backend } from "./backend";

export const sample = {
  spec: T.DotSpec.parse(contract.DotSpec),
  dot: T.Dot.parse(contract.Dot),
  run: T.Run.parse(contract.Run),
  event: T.RunEventRecord.parse(contract.RunEventRecord),
  approval: T.Approval.parse(contract.Approval),
  status: T.Status.parse(contract.Status),
  settings: T.Settings.parse(contract.Settings),
  overview: T.DotOverview.parse(contract.DotOverview),
  inbox: T.Inbox.parse(contract.Inbox),
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
  startupError: T.StartupError.parse(contract.StartupError),
};

/// What `new_dot_spec` answers: the core's defaults with nothing filled in.
export const blankSpec: T.DotSpec = {
  name: "",
  instructions: "",
  engine: "claude",
  model: "sonnet",
  endpoint_url: null,
  workdir: "",
  workspace_mode: "worktree",
  schedule: null,
  policy: sample.dot.policy,
  mcp_servers: null,
  use_user_settings: false,
  max_turns: 40,
  timeout_secs: 1800,
  approval_wait_secs: 240,
  enabled: true,
};

/// A status with nothing to warn about.
export const quietStatus: T.Status = {
  ...sample.status,
  claude: { ...sample.status.claude, version: sample.status.claude.tested_version, mismatch: false },
};

/// The answers every screen's frame needs: a started app with nothing to warn about, no dots
/// and an empty inbox. A test replaces what it cares about with `backend.on`.
export function installDefaults(): void {
  backend.on("startup_error", () => null);
  backend.on("status", () => quietStatus);
  backend.on("inbox", () => ({ approvals: [], review_runs: [] }));
  backend.on("dot_overview", () => []);
  backend.on("list_dots", () => []);
  backend.on("get_settings", () => sample.settings);
  backend.on("new_dot_spec", () => blankSpec);
  backend.on("list_runs", () => []);
  backend.on("webhook_url", (args) => `http://127.0.0.1:47321/dots/${String(args.id)}/trigger`);
  backend.on("preview_schedule", () => [
    "2026-10-13T02:00:00+01:00",
    "2026-10-14T02:00:00+01:00",
    "2026-10-15T02:00:00+01:00",
  ]);
}
