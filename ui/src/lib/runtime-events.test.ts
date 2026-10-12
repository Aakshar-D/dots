import { describe, expect, it } from "vitest";

import type { RunEventRecord } from "@/api/types";
import { createQueryClient, keys, mergeEvents } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { sample } from "@/test/fixtures";

const event = (seq: number): RunEventRecord => ({ ...sample.event, seq });
const stale = (qc: ReturnType<typeof createQueryClient>, key: readonly unknown[]) =>
  qc.getQueryState(key)?.isInvalidated;

describe("runtime events and the query cache", () => {
  it("merges events by seq without duplicates", () => {
    const have = [event(1), event(2)];
    expect(mergeEvents(have, [event(2), event(4), event(3)]).map((e) => e.seq)).toEqual([
      1, 2, 3, 4,
    ]);
    // Nothing new: the same array, so nothing re-renders.
    expect(mergeEvents(have, [event(1)])).toBe(have);
  });

  it("appends a run event to a loaded list and leaves an unloaded one alone", () => {
    const qc = createQueryClient();
    const key = keys.events(sample.event.run_id);
    applyRuntimeEvent(qc, { type: "run_event", event: event(1) });
    expect(qc.getQueryData(key)).toBeUndefined();

    qc.setQueryData(key, [event(1)]);
    applyRuntimeEvent(qc, { type: "run_event", event: event(2) });
    applyRuntimeEvent(qc, { type: "run_event", event: event(2) });
    expect(qc.getQueryData<RunEventRecord[]>(key)?.map((e) => e.seq)).toEqual([1, 2]);
  });

  it("stores an updated run and marks what lists it stale", () => {
    const qc = createQueryClient();
    const run = { ...sample.run, status: "failed" as const };
    const lists = [
      keys.thread(run.root_run_id),
      keys.runs(run.dot_id),
      keys.runs(null),
      keys.diff(run.id),
      keys.overview,
      keys.inbox,
    ];
    for (const key of lists) qc.setQueryData(key, []);
    qc.setQueryData(keys.settings, sample.settings);

    applyRuntimeEvent(qc, { type: "run_updated", run });
    expect(qc.getQueryData(keys.run(run.id))).toEqual(run);
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    expect(stale(qc, keys.settings)).toBe(false);
  });

  it("marks the inbox and the run's approvals stale on an approval event", () => {
    const qc = createQueryClient();
    const mine = keys.approvals(sample.approval.run_id);
    const other = keys.approvals("another-run");
    for (const key of [keys.inbox, mine, other]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "approval_requested", approval: sample.approval });
    expect([stale(qc, keys.inbox), stale(qc, mine), stale(qc, other)]).toEqual([
      true,
      true,
      false,
    ]);
  });
});
