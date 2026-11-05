import { describe, expect, it } from "vitest";

import { Step, WorkflowSpec } from "@/api/types";

import {
  formatSpend,
  newWorkflowSpec,
  taskLine,
  uniqueName,
  workflowRunStatusLabel,
} from "./format";

describe("the team page's words", () => {
  it("names a workflow run by its task's first line", () => {
    expect(taskLine("  Fix the tray icon.\r\nIt is invisible.")).toBe("Fix the tray icon.");
    expect(taskLine("One line")).toBe("One line");
    expect(taskLine("")).toBe("");
  });

  it("says what a run spent, and what a waiting run waits for when it is known", () => {
    expect(formatSpend({ spent_runs: 1, spent_tokens: 950 })).toBe("1 run · 950 tokens");
    expect(formatSpend({ spent_runs: 4, spent_tokens: 182_000 })).toBe("4 runs · 182k tokens");
    expect(workflowRunStatusLabel({ status: "waiting", waiting_on: "check" })).toBe(
      "Needs your check",
    );
    expect(workflowRunStatusLabel({ status: "waiting", waiting_on: "send_backs" })).toBe(
      "Hit its send-back limit",
    );
    expect(workflowRunStatusLabel({ status: "waiting" })).toBe("Waiting");
    expect(workflowRunStatusLabel({ status: "cancelled", waiting_on: null })).toBe("Cancelled");
  });

  it("gives a new workflow a name the team has not used, without case", () => {
    expect(uniqueName("New workflow", ["Nightly"])).toBe("New workflow");
    expect(uniqueName("New workflow", ["new workflow ", "New workflow 2"])).toBe("New workflow 3");
  });

  it("makes a new workflow the UI's own schemas read", () => {
    const spec = newWorkflowSpec("New workflow", "01J0DOT");
    expect(WorkflowSpec.parse(spec)).toEqual(spec);
    expect(Step.parse(spec.definition.steps[0])).toMatchObject({
      kind: "agent",
      member: "01J0DOT",
    });
  });
});
