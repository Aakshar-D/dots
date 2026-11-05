import { describe, expect, it } from "vitest";

import fixture from "./__fixtures__/contract.json";
import * as T from "./types";

/// Every step of `steps`, those inside branches included, in order.
function all(steps: T.Step[]): T.Step[] {
  return steps.flatMap((s) =>
    s.kind === "branch"
      ? [s, ...all(s.paths.flatMap((p) => p.steps)), ...all(s.otherwise)]
      : [s],
  );
}

const sorted = (values: Iterable<string>) => [...new Set(values)].sort();

const agent = (id: string): T.Step => ({
  id,
  name: "Work",
  kind: "agent",
  member: "01J0DOT",
  job: "Do the task.",
  can_send_back_to: [],
  outcomes: [],
  delegate: null,
});

/// A branch on outcome whose "fixed" path holds a branch on project: two levels of paths.
const nested: T.Step = {
  id: "outer",
  name: "Fixed?",
  kind: "branch",
  on: "outcome",
  paths: [
    {
      label: "fixed",
      steps: [
        {
          id: "inner",
          name: "Which project",
          kind: "branch",
          on: "project",
          paths: [{ label: "Dots", steps: [agent("dots")] }],
          otherwise: [
            { id: "ship", name: "Ship", kind: "deploy", mode: "pr_merge", merge_method: "rebase" },
          ],
        },
      ],
    },
  ],
  otherwise: [agent("design")],
};

describe("the teams and workflows schemas", () => {
  it("has a workflow sample of every step kind, deploy mode and trigger kind", () => {
    const workflows = T.Workflow.array().parse(fixture.Workflow);
    const steps = workflows.flatMap((w) => all(w.definition.steps));
    expect(sorted(steps.map((s) => s.kind))).toEqual(["agent", "branch", "check", "deploy"]);
    const modes = T.DeployStep.options.map((o) => o.shape.mode.value);
    expect(sorted(steps.flatMap((s) => (s.kind === "deploy" ? [s.mode] : [])))).toEqual(
      sorted(modes),
    );
    const triggers = T.Trigger.options.map((o) => o.shape.kind.value);
    expect(sorted(workflows.map((w) => w.definition.trigger.kind))).toEqual(sorted(triggers));
    // A delegate, a check that names where it sends back and one that does not, and a branch
    // with an otherwise and one without.
    expect(steps.some((s) => s.kind === "agent" && s.delegate !== null)).toBe(true);
    const checks = steps.flatMap((s) => (s.kind === "check" ? [s.send_back_to] : []));
    expect(checks).toEqual(expect.arrayContaining(["build", null]));
    const branches = steps.flatMap((s) => (s.kind === "branch" ? [s] : []));
    expect(branches.map((b) => b.otherwise.length > 0).sort()).toEqual([false, true]);
    // The workflow the builder edits is the spec it saves: the same fields, flattened.
    expect(T.WorkflowSpec.array().parse(fixture.WorkflowSpec)).toEqual(
      workflows.map((w) => ({ name: w.name, definition: w.definition, enabled: w.enabled })),
    );
  });

  it("has a workflow run in each wait, with members, decisions, a deploy and a held send-back", () => {
    const runs = T.WorkflowRun.array().parse(fixture.WorkflowRun);
    expect(sorted(runs.flatMap((r) => (r.waiting_on ? [r.waiting_on] : [])))).toEqual(
      sorted(T.WaitingOn.options),
    );
    const steps = runs.flatMap((r) => r.steps);
    expect(steps.some((s) => (s.handoff?.members.length ?? 0) > 0)).toBe(true);
    expect(steps.some((s) => s.handoff?.members.some((m) => m.branch === null))).toBe(true);
    const decisions = steps.flatMap((s) => (s.decision ? [s.decision] : []));
    expect(decisions.map((d) => ("decision" in d ? d.decision : "path")).sort()).toEqual(
      expect.arrayContaining(["approved", "path", "sent_back"]),
    );
    const results = steps.flatMap((s) => (s.deploy_result ? [s.deploy_result] : []));
    expect(results.some((d) => d.ready)).toBe(true);
    expect(results.some((d) => d.error !== undefined)).toBe(true);
    expect(results.some((d) => d.branches.some((b) => b.pr_url !== null && b.auto_merge))).toBe(
      true,
    );
    expect(runs.some((r) => r.send_back?.held === true)).toBe(true);
    expect(steps.some((s) => s.send_back !== null)).toBe(true);
  });

  it("reads a branch step inside a branch's path, and refuses an unknown field at any depth", () => {
    expect(T.Step.parse(nested)).toEqual(nested);
    expect(all([T.Step.parse(nested)]).map((s) => s.id)).toEqual([
      "outer",
      "inner",
      "dots",
      "ship",
      "design",
    ]);
    const deep = structuredClone(nested) as Extract<T.Step, { kind: "branch" }>;
    const inner = deep.paths[0].steps[0] as Extract<T.Step, { kind: "branch" }>;
    Object.assign(inner.paths[0].steps[0], { extra: 1 });
    expect(T.Step.safeParse(deep).success).toBe(false);
  });

  it("refuses a step of a kind, a deploy of a mode or a trigger it does not know", () => {
    expect(T.Step.safeParse({ ...agent("a"), kind: "wait" }).success).toBe(false);
    expect(T.Step.safeParse({ id: "s", name: "Ship", kind: "deploy", mode: "tag" }).success).toBe(
      false,
    );
    // A mode's own fields belong to it alone.
    const push = { id: "s", name: "Ship", kind: "deploy", mode: "push" };
    expect(T.Step.safeParse({ ...push, merge_method: "squash" }).success).toBe(false);
    expect(T.Trigger.safeParse({ kind: "email" }).success).toBe(false);
  });

  it("reads a deploy that succeeded without an error, and an approval kept before its tips", () => {
    expect(T.DeployResult.parse({ branches: [], ready: true })).toEqual({
      branches: [],
      ready: true,
    });
    expect(T.StepDecision.parse({ decision: "approved", note: null })).toEqual({
      decision: "approved",
      note: null,
    });
    expect(T.StepDecision.safeParse({ decision: "approved", note: null, by: "x" }).success).toBe(
      false,
    );
  });

  it("knows what the builder's live check and the check page answer", () => {
    const problems = T.WorkflowProblem.array().parse(fixture.WorkflowProblem);
    expect(problems.map((p) => p.step_id)).toEqual(["work", null]);
    const view = T.CheckView.parse(fixture.CheckView);
    // Approve sends back the branches and commits the page showed.
    expect(T.CheckTip.array().parse(fixture.CheckTip)).toEqual(
      view.branches.map(({ branch, commit }) => ({ branch, commit })),
    );
  });
});
