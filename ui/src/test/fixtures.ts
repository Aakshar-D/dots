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
  /// What the core offers to save with `approval`, narrowest first.
  rules: T.RuleCandidate.array().parse(contract.RuleCandidate),
  status: T.Status.parse(contract.Status),
  settings: T.Settings.parse(contract.Settings),
  overview: T.DotOverview.parse(contract.DotOverview),
  /// A stdio server with a changed tool list and a missing variable; a plugin's HTTP server; a
  /// skill with a problem; a plugin with every part.
  library: T.LibraryEntry.array().parse(contract.LibraryEntry),
  /// The same items without the extras of a list entry: what `library_add` answers.
  libraryItems: T.LibraryItem.array().parse(contract.LibraryItem),
  scannedSkills: T.ScannedSkill.array().parse(contract.ScannedSkill),
  contextDocuments: T.ContextDocuments.parse(contract.ContextDocuments),
  installedPlugins: T.InstalledPlugin.array().parse(contract.InstalledPlugin),
  probe: T.McpProbe.parse(contract.McpProbe),
  inbox: T.Inbox.parse(contract.Inbox),
  diff: T.WorktreeDiff.parse(contract.WorktreeDiff),
  startupError: T.StartupError.parse(contract.StartupError),
  /// One schedule of each kind: once; every with an anchor and a count; every without an
  /// anchor; daily until a date; weekly; cron.
  schedules: T.Schedule.array().parse(contract.Schedule),
  /// An open question with the choices "yes" and "no", an answered one, a dismissed one, and a
  /// read note of Dots' own.
  messages: T.Message.array().parse(contract.Message),
  /// A pending follow-up and a fired one.
  followUps: T.FollowUp.array().parse(contract.FollowUp),
};

/// What `new_dot_spec` answers: the core's defaults with nothing filled in.
export const blankSpec: T.DotSpec = T.DotSpec.parse(contract.NewDotSpec);

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
  backend.on("update_note", () => ({ state: "none" }));
  backend.on("inbox", () => ({ approvals: [], review_runs: [], attention: [], tools_changed: [] }));
  // An approval card asks for these when it shows; by default the core offers no rule.
  backend.on("approval_rule_candidates", () => []);
  backend.on("dot_overview", () => []);
  backend.on("list_dots", () => []);
  backend.on("get_settings", () => sample.settings);
  backend.on("library_list", () => []);
  backend.on("installed_claude_plugins", () => []);
  backend.on("environment_variables", (args) =>
    (args.names as string[]).map((name) => ({ name, set: true })),
  );
  backend.on("new_dot_spec", () => blankSpec);
  backend.on("list_runs", () => []);
  // No dot has checked in or scheduled anything. A question on screen is marked read.
  backend.on("messages_list", () => []);
  backend.on("follow_ups_list", () => []);
  backend.on("message_mark_read", (args) => ({
    ...sample.messages[0],
    id: String(args.id),
    read_at: sample.run.queued_at,
  }));
  backend.on("webhook_url", (args) => `http://127.0.0.1:47321/dots/${String(args.id)}/trigger`);
  backend.on("preview_schedule", () => [
    "2026-10-13T02:00:00+01:00",
    "2026-10-14T02:00:00+01:00",
    "2026-10-15T02:00:00+01:00",
  ]);
}
