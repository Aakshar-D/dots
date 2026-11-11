import type { GitAction, Guardrails, PolicyCeiling, Preset } from "@/api/types";

/// A preset's name as the core's refusals spell it.
export const PRESET_NAMES: Record<Preset, string> = {
  "read-only": "Read-only",
  sandboxed: "Sandboxed",
  trusted: "Trusted",
  custom: "Custom",
};

/// The presets a Member's dot may be held to (guardrails spec section 3.1: not `custom`).
export const MAX_PRESETS: Preset[] = ["read-only", "sandboxed", "trusted"];

/// A git action in the words of the core's refusals (`GitAction::label`). The Guardrails tab
/// takes them from `GuardrailsView.git_actions`; the dot editor, which has no view, uses these.
export const GIT_LABELS: Record<GitAction, string> = {
  push_main: "pushing to main or master",
  force_push: "force pushing",
  delete_branch: "deleting a branch",
  tags: "making or pushing tags",
  merge_pr: "merging a pull request",
  release: "publishing a GitHub release",
};

/// "Pushing to main or master": a git action's label as a list item or checkbox shows it.
export function capitalized(label: string): string {
  return label.charAt(0).toUpperCase() + label.slice(1);
}

/// The ceiling in a sentence for the dot editor, or null when none applies: "Members' dots in
/// Payments can go up to Sandboxed." for a Member, and the always-ask rule for an Admin.
export function ceilingLine(ceiling: PolicyCeiling): string | null {
  const where = ceiling.workspace_name ?? "this workspace";
  if (ceiling.applies_as === "member" && ceiling.max_preset) {
    return `Members' dots in ${where} can go up to ${PRESET_NAMES[ceiling.max_preset]}. Protected calls are refused; an Admin can make them.`;
  }
  if (ceiling.applies_as === "admin") {
    return `${where} has guardrails: your dots always ask before a protected call.`;
  }
  return null;
}

/// "3 paths, 6 git actions, 16 commands": what guardrails protect, counted.
export function protectedCount(g: Guardrails): string {
  const n = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`;
  return [
    n(g.protected_paths.length, "path", "paths"),
    n(g.protected_git.length, "git action", "git actions"),
    n(g.protected_commands.length, "command", "commands"),
  ].join(", ");
}

/// The lines of a text box: trimmed, without empty ones.
export function lines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");
}
