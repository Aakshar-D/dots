import { describe, expect, it } from "vitest";

import type { RunEventRecord } from "@/api/types";
import { planProgress, planSteps, runPlans, runTimeline, worthShowing } from "@/lib/run-plan";

let seq = 0;
const at = (s: number) => new Date(Date.UTC(2026, 10, 6, 10, 0, s)).toISOString();
/// An event `s` seconds after 10:00 UTC; `seq` counts up unless given.
const ev = (data: Record<string, unknown>, s = 0, n = ++seq): RunEventRecord => ({
  run_id: "r1",
  seq: n,
  ts: at(s),
  kind: String(data.kind),
  data,
});
const todo = (content: string, status: string, activeForm = `${content}ing`) => ({
  content,
  status,
  activeForm,
});
const plan = (id: string, todos: unknown, parent?: string) =>
  ev({ kind: "tool_call", id, tool: "TodoWrite", input: { todos }, ...(parent ? { parent } : {}) });
const ok = (id: string, s = 0) => ev({ kind: "tool_result", id, output: "ok", is_error: false }, s);
const call = (id: string, tool: string, input: unknown, s = 0, parent?: string) =>
  ev({ kind: "tool_call", id, tool, input, ...(parent ? { parent } : {}) }, s);

describe("a run's plan", () => {
  it("is the newest to-do list the sol wrote, with each step's state", () => {
    const plans = runPlans([
      plan("p1", [todo("Read", "in_progress"), todo("Fix", "pending")]),
      ok("p1"),
      plan("p2", [todo("Read", "completed"), todo("Fix", "in_progress", "Fixing the bug")]),
      ok("p2"),
    ]);
    expect(plans).toHaveLength(1);
    expect(plans[0]!.owner).toEqual({ kind: "dot" });
    expect(plans[0]!.steps).toEqual([
      { content: "Read", activeForm: "Reading", status: "completed" },
      { content: "Fix", activeForm: "Fixing the bug", status: "in_progress" },
    ]);
    expect(planProgress(plans[0]!.steps)).toEqual({ done: 1, total: 2 });
  });

  it("goes by seq, not by the order the events came in", () => {
    const tw = (id: string, status: string, n: number) =>
      ev({ kind: "tool_call", id, tool: "TodoWrite", input: { todos: [todo("A", status)] } }, 0, n);
    const older = tw("p1", "pending", 1);
    const newer = tw("p2", "completed", 2);
    expect(runPlans([newer, older])[0]!.steps[0]!.status).toBe("completed");
  });

  it("counts an event delivered twice once", () => {
    const one = plan("p1", [todo("A", "pending")]);
    expect(runPlans([one, one])).toHaveLength(1);
  });

  it("keeps the last good list when a call is not a to-do list or failed", () => {
    const plans = runPlans([
      plan("p1", [todo("A", "in_progress")]),
      // Not a list at all: changes nothing.
      ev({ kind: "tool_call", id: "p2", tool: "TodoWrite", input: { todos: "nope" } }),
      ev({ kind: "tool_call", id: "p3", tool: "TodoWrite", input: null }),
      // A call the CLI refused.
      plan("p4", [todo("B", "pending")]),
      ev({ kind: "tool_result", id: "p4", output: "bad input", is_error: true }),
    ]);
    expect(plans[0]!.steps.map((s) => s.content)).toEqual(["A"]);
  });

  it("leaves out items without words and reads an unknown state as not started", () => {
    expect(
      planSteps({
        todos: [
          { content: "  ", status: "pending" },
          { status: "completed" },
          42,
          { content: "Ship", status: "cancelled" },
          { content: "Test" },
        ],
      }),
    ).toEqual([
      { content: "Ship", activeForm: "Ship", status: "pending" },
      { content: "Test", activeForm: "Test", status: "pending" },
    ]);
    expect(planSteps({})).toBeNull();
  });

  it("is cleared by an empty list", () => {
    expect(runPlans([plan("p1", [todo("A", "pending")]), plan("p2", [])])).toEqual([]);
  });

  it("gives a helper's list to that helper, after the sol's own", () => {
    const plans = runPlans([
      call("task1", "Task", { description: "Check the logs", prompt: "…" }),
      plan("h1", [todo("Open logs", "in_progress")], "task1"),
      plan("p1", [todo("Plan", "completed"), todo("Wait for helper", "in_progress")]),
    ]);
    expect(plans.map((p) => p.owner)).toEqual([
      { kind: "dot" },
      { kind: "helper", id: "task1", label: "Check the logs" },
    ]);
    // The helper's list did not replace the dot's.
    expect(plans[0]!.steps).toHaveLength(2);
    expect(plans[1]!.steps[0]!.content).toBe("Open logs");
  });

  it("gives a call whose helper the run does not know to the sol", () => {
    const plans = runPlans([plan("p1", [todo("A", "pending")], "unknown")]);
    expect(plans[0]!.owner).toEqual({ kind: "dot" });
  });

  it("finds none in a run that kept no plan", () => {
    expect(runPlans([call("r", "Read", { file_path: "a" }), ok("r")])).toEqual([]);
  });
});

describe("a run's timeline", () => {
  it("groups consecutive calls, with times and when the last one ended", () => {
    const entries = runTimeline(
      [
        call("a", "Read", { file_path: "a.rs" }, 0),
        call("b", "Read", { file_path: "b.rs" }, 1),
        ok("a", 2),
        ok("b", 5),
        ev({ kind: "assistant_text", text: "Now the tests." }, 6),
        call("c", "Bash", { command: "cargo test" }, 7),
        ok("c", 70),
      ],
      false,
    );
    expect(entries.map((e) => [e.text, e.status, e.start, e.end])).toEqual([
      ["Read 2 files", "done", at(0), at(5)],
      ["Ran a command", "done", at(7), at(70)],
    ]);
    expect(entries[0]!.calls.map((c) => c.result?.output)).toEqual(["ok", "ok"]);
  });

  it("shows a call still going as running, and as having no result once the run ended", () => {
    const events = [call("a", "Bash", { command: "sleep" })];
    expect(runTimeline(events, true)[0]).toMatchObject({ status: "running", end: null });
    expect(runTimeline(events, false)[0]).toMatchObject({ status: "stopped" });
  });

  it("says a failed call failed and a parked one waits for approval", () => {
    const failed = runTimeline(
      [call("a", "Bash", {}), ev({ kind: "tool_result", id: "a", output: "x", is_error: true })],
      false,
    );
    expect(failed[0]!.status).toBe("failed");
    const parked = runTimeline(
      [
        call("a", "Bash", {}),
        ev({
          kind: "tool_result",
          id: "a",
          output: "Queued for human approval #1.",
          is_error: true,
        }),
      ],
      true,
    );
    expect(parked[0]!.status).toBe("waiting");
  });

  it("puts questions, helpers, members and workflows on lines of their own", () => {
    const entries = runTimeline(
      [
        call("r", "Read", {}),
        call("q", "mcp__dots__ask", { question: "Which branch?" }),
        call("t", "Task", { description: "Check the logs" }),
        call("m", "mcp__dots__start_member", { member: "Ada", task: "Write the docs\nmore" }),
        call("w", "mcp__dots__start_workflow", { workflow: "Release", project: "p", task: "t" }),
        call("r2", "Read", {}),
      ],
      true,
    );
    expect(entries.map((e) => [e.kind, e.text])).toEqual([
      ["tools", "Read a file"],
      ["checkin", "Asked you: Which branch?"],
      ["helper", "Handed to a helper: Check the logs"],
      ["member", "Handed to Ada: Write the docs"],
      ["workflow", "Started the workflow Release"],
      ["tools", "Read a file"],
    ]);
  });

  it("names a helper's steps by the helper, apart from the sol's", () => {
    const entries = runTimeline(
      [
        call("t", "Task", { description: "Check the logs" }),
        call("a", "Read", {}, 0, "t"),
        call("b", "Grep", {}, 0, "t"),
        call("c", "Read", {}),
      ],
      true,
    );
    expect(entries.map((e) => [e.owner.kind, e.text])).toEqual([
      ["dot", "Handed to a helper: Check the logs"],
      ["helper", "Read a file · Searched the files"],
      ["dot", "Read a file"],
    ]);
  });

  it("marks each plan step once, when it is done, and leaves plan updates out", () => {
    const entries = runTimeline(
      [
        plan("p1", [todo("Read", "in_progress"), todo("Fix", "pending")]),
        call("a", "Read", {}),
        plan("p2", [todo("Read", "completed"), todo("Fix", "in_progress")]),
        plan("p3", [todo("Read", "completed"), todo("Fix", "completed")]),
      ],
      false,
    );
    expect(entries.map((e) => e.text)).toEqual(["Read a file", "Done: Read", "Done: Fix"]);
  });
});

describe("whether a run shows a card", () => {
  it("shows one for a plan, a hand-off or several steps, and none for a quick answer", () => {
    const quick = runTimeline([call("a", "Read", {}), ok("a")], false);
    expect(worthShowing([], quick)).toBe(false);
    expect(worthShowing([], [])).toBe(false);
    const busy = runTimeline(
      [
        call("a", "Read", {}),
        call("b", "Read", {}),
        ev({ kind: "assistant_text", text: "…" }),
        call("c", "Bash", {}),
      ],
      false,
    );
    expect(worthShowing([], busy)).toBe(true);
    const handed = runTimeline([call("t", "Task", { description: "x" })], true);
    expect(worthShowing([], handed)).toBe(true);
    const planned = runPlans([plan("p", [todo("A", "pending")])]);
    expect(worthShowing(planned, [])).toBe(true);
  });
});
