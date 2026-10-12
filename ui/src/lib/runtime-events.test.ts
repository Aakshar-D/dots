import { QueryObserver } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import type { RunEventRecord } from "@/api/types";
import { createQueryClient, eventsQuery, keys, mergeEvents } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

type Client = ReturnType<typeof createQueryClient>;

const event = (seq: number): RunEventRecord => ({ ...sample.event, seq });
const stale = (qc: Client, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

const runId = sample.event.run_id;
const key = keys.events(runId);
const seqs = (qc: Client) => qc.getQueryData<RunEventRecord[]>(key)?.map((e) => e.seq);
const arrive = (qc: Client, seq: number) =>
  applyRuntimeEvent(qc, { type: "run_event", event: event(seq) });
/// The arguments of every `list_run_events` call so far.
const asked = () => backend.callsTo("list_run_events");

/// Shows the run's events as a screen does, so a list marked stale is fetched again.
const watch = (qc: Client) => new QueryObserver(qc, eventsQuery(qc, runId)).subscribe(() => {});

/// Answers `list_run_events` from what the backend holds: the events after `afterSeq`.
const holding = (stored: RunEventRecord[]) => (args: Record<string, unknown>) =>
  stored.filter((e) => e.seq > (args.afterSeq as number));

/// An answer the test gives later, as a backend that read its rows before a newer event.
function later<T>() {
  let give!: (value: T) => void;
  const answer = new Promise<T>((resolve) => (give = resolve));
  return { answer, give };
}

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

describe("a run's events while their list loads or refetches", () => {
  it("keeps an event patched in while a refetch is in flight", async () => {
    const qc = createQueryClient();
    qc.setQueryData(key, [event(1), event(2)]);
    const unwatch = watch(qc);
    const read = later<RunEventRecord[]>();
    backend.on("list_run_events", () => read.answer);

    const refetch = qc.refetchQueries({ queryKey: key });
    await vi.waitFor(() => expect(asked()).toEqual([{ runId, afterSeq: 2 }]));
    arrive(qc, 3);
    // The backend read its rows before it stored event 3.
    read.give([]);
    await refetch;

    expect(seqs(qc)).toEqual([1, 2, 3]);
    unwatch();
  });

  it("asks again for an event that arrived during the first load", async () => {
    const qc = createQueryClient();
    const stored = [event(1), event(2)];
    const read = later<RunEventRecord[]>();
    backend.on("list_run_events", (args) =>
      asked().length === 1 ? read.answer : holding(stored)(args),
    );
    const unwatch = watch(qc);
    await vi.waitFor(() => expect(asked()).toEqual([{ runId, afterSeq: 0 }]));

    // The first load read events 1 and 2; 3 and 4 are stored and emitted before it answers.
    const firstRead = [...stored];
    stored.push(event(3), event(4));
    arrive(qc, 3);
    arrive(qc, 4);
    expect(seqs(qc)).toBeUndefined();
    read.give(firstRead);

    // No further event arrives: the list catches up by itself, with one more question.
    await vi.waitFor(() => expect(seqs(qc)).toEqual([1, 2, 3, 4]));
    expect(asked()).toEqual([
      { runId, afterSeq: 0 },
      { runId, afterSeq: 2 },
    ]);
    unwatch();
  });

  it("marks a list nobody shows stale when an event arrived during its load", async () => {
    const qc = createQueryClient();
    const read = later<RunEventRecord[]>();
    backend.on("list_run_events", () => read.answer);
    const load = qc.prefetchQuery(eventsQuery(qc, runId));
    await vi.waitFor(() => expect(asked()).toHaveLength(1));

    arrive(qc, 3);
    read.give([event(1), event(2)]);
    await load;

    // Stale, so the screen that shows it next asks for what is after event 2.
    await vi.waitFor(() => expect(stale(qc, key)).toBe(true));
    expect(seqs(qc)).toEqual([1, 2]);
    expect(asked()).toHaveLength(1);
  });

  it("fills a gap from the backend instead of appending past it", async () => {
    const qc = createQueryClient();
    qc.setQueryData(key, [event(1), event(2)]);
    backend.on("list_run_events", holding([event(1), event(2), event(3), event(4)]));
    const unwatch = watch(qc);

    arrive(qc, 4);
    expect(seqs(qc)).toEqual([1, 2]);

    await vi.waitFor(() => expect(seqs(qc)).toEqual([1, 2, 3, 4]));
    expect(asked()).toEqual([{ runId, afterSeq: 2 }]);
    unwatch();
  });

  it("changes and fetches nothing for an event the list already holds", () => {
    const qc = createQueryClient();
    const have = [event(1), event(2)];
    qc.setQueryData(key, have);
    const unwatch = watch(qc);

    arrive(qc, 2);
    arrive(qc, 1);

    expect(qc.getQueryData(key)).toBe(have);
    expect(stale(qc, key)).toBe(false);
    expect(asked()).toEqual([]);
    unwatch();
  });

  it("appends the next event in order without a fetch", () => {
    const qc = createQueryClient();
    qc.setQueryData(key, []);
    const unwatch = watch(qc);

    arrive(qc, 1);
    arrive(qc, 2);

    expect(seqs(qc)).toEqual([1, 2]);
    expect(stale(qc, key)).toBe(false);
    expect(asked()).toEqual([]);
    unwatch();
  });
});
