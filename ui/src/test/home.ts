// A dot and its runs for the home's tests: the fake backend answers the home's commands from
// the runs, the events and the approvals a test gives, as the core would.
import type { Approval, DotOverview, Run, RunEventRecord } from "@/api/types";

import { backend } from "./backend";
import { sample } from "./fixtures";

export const dot = { ...sample.dot, schedule: null };

/// The dot's row in the column: idle, with no run.
export const overview: DotOverview = {
  dot,
  active_run: null,
  last_run: null,
  schedule_state: null,
};

/// A root run of the dot queued at `at` ("HH:MM" on 9 October, UTC): a chat run asking
/// "question <id>" that succeeded, unless `over` says otherwise.
export const chat = (id: string, at: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  dot_id: dot.id,
  root_run_id: id,
  parent_run_id: null,
  trigger: "chat",
  branch: null,
  summary: null,
  payload: { message: `question ${id}` },
  queued_at: `2026-10-09T${at}:00.000Z`,
  ...over,
});

/// A scheduled root run of the dot queued at `at` that succeeded with `summary`.
export const scheduled = (id: string, at: string, over: Partial<Run> = {}): Run =>
  chat(id, at, { trigger: "schedule", payload: null, summary: `Summary of ${id}`, ...over });

/// A chat run that continued `parent`, queued at `at`.
export const reply = (id: string, parent: Run, at: string, over: Partial<Run> = {}): Run =>
  chat(id, at, {
    root_run_id: parent.root_run_id,
    parent_run_id: parent.id,
    payload: { message: `reply ${id}` },
    ...over,
  });

/// The dot said `text` in run `runId`.
export const said = (runId: string, seq: number, text: string): RunEventRecord => ({
  run_id: runId,
  seq,
  ts: sample.run.queued_at,
  kind: "assistant_text",
  data: { kind: "assistant_text", text },
});

/// A backend holding the dot and `runs` (changed in place by the test or by `chat`, which
/// appends a queued run as the core does). Answers the column, the runs, the threads, the
/// events and the approvals.
export function home(runs: Run[], events: RunEventRecord[] = [], approvals: Approval[] = []) {
  const all = [...runs];
  backend.on("dot_overview", () => [overview]);
  backend.on("list_dots", () => [dot]);
  backend.on("list_runs", () => [...all].reverse());
  backend.on("run_thread", (args) => {
    const root = all.find((r) => r.id === args.runId)?.root_run_id;
    return all.filter((r) => r.root_run_id === root);
  });
  backend.on("list_run_events", (args) =>
    events.filter((e) => e.run_id === args.runId && e.seq > (args.afterSeq as number)),
  );
  backend.on("approvals_for_run", (args) => approvals.filter((a) => a.run_id === args.runId));
  backend.on("chat", (args) => {
    const parent = all.find((r) => r.id === args.parentRunId);
    const id = `run-${all.length + 1}`;
    // Each one a minute after the one before.
    const run = chat(id, `15:${String(all.length).padStart(2, "0")}`, {
      status: "queued",
      ended_at: null,
      payload: { message: args.message },
      parent_run_id: parent?.id ?? null,
      root_run_id: parent?.root_run_id ?? id,
    });
    all.push(run);
    return run;
  });
  return all;
}
