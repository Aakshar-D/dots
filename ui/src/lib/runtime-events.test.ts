import { QueryObserver } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { commands } from "@/api/commands";
import type { DotOverview, Inbox, RunEventRecord } from "@/api/types";
import {
  createQueryClient,
  eventsQuery,
  invalidate,
  invalidateDots,
  keys,
  mergeEvents,
} from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend, later } from "@/test/backend";
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

  it("marks what shows an updated run stale", () => {
    const qc = createQueryClient();
    const run = { ...sample.run, status: "failed" as const };
    const lists = [
      keys.thread(run.root_run_id),
      keys.runs(run.dot_id),
      [...keys.runs(run.dot_id), "chat"],
      keys.diff(run.id),
      keys.overview,
      keys.inbox,
      // A run that ends takes its pending approvals with it, and no approval event says so.
      keys.approvals(run.id),
    ];
    const untouched = [keys.settings, keys.approvals("another-run")];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "run_updated", run });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
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

  it("marks the dots, the overview and the inbox stale on a schedule update", () => {
    const qc = createQueryClient();
    const lists = [keys.dots, keys.overview, keys.inbox];
    const untouched = [keys.dot(sample.dot.id), keys.runs(sample.dot.id), keys.settings];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "schedule_updated", dot_id: sample.dot.id });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("marks the Library, the inbox and what shows a dot's uses stale on a Library update", () => {
    const qc = createQueryClient();
    // Removing an item takes it out of the dots that use it; a changed tool list is an Inbox
    // group.
    const lists = [keys.library, keys.inbox, keys.dots, keys.dot(sample.dot.id), keys.overview];
    const untouched = [keys.settings, keys.runs(sample.dot.id), keys.installedPlugins];
    for (const key of [...lists, ...untouched]) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, { type: "library_updated", item_id: "01J0SRV" });
    for (const key of lists) expect(stale(qc, key), key.join("/")).toBe(true);
    for (const key of untouched) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("leaves the caches to the schedule update that comes with a schedule's attention", () => {
    const qc = createQueryClient();
    const all = [keys.dots, keys.overview, keys.inbox, keys.dot(sample.dot.id)];
    for (const key of all) qc.setQueryData(key, []);

    applyRuntimeEvent(qc, {
      type: "schedule_attention",
      dot_id: sample.dot.id,
      run_id: sample.run.id,
      error: "model server unreachable",
    });
    for (const key of all) expect(stale(qc, key), key.join("/")).toBe(false);
  });

  it("refetches the approvals of the approval's own run when Dots performs its call", () => {
    const qc = createQueryClient();
    // The call is performed by a resume run; the approval belongs to that run's parent.
    const parent = keys.approvals(sample.approval.run_id);
    const resume = keys.approvals("the-resume-run");
    for (const key of [keys.inbox, parent, resume]) qc.setQueryData(key, []);

    const performed = { ...sample.approval, self_apply: true, apply_state: "applied" as const };
    applyRuntimeEvent(qc, { type: "approval_updated", approval: performed });
    expect([stale(qc, keys.inbox), stale(qc, parent), stale(qc, resume)]).toEqual([
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

  it("marks a list nobody shows stale when a gap opened during its refetch", async () => {
    const qc = createQueryClient();
    qc.setQueryData(key, [event(1), event(2)]);
    const read = later<RunEventRecord[]>();
    backend.on("list_run_events", () => read.answer);
    // The screen asks for more and closes: the fetch goes on with nobody showing the list.
    const unwatch = watch(qc);
    const refetch = qc.refetchQueries({ queryKey: key });
    await vi.waitFor(() => expect(asked()).toEqual([{ runId, afterSeq: 2 }]));
    unwatch();

    // The backend read event 3 only; 4 and 5 are stored before it answers, and 5 arrives.
    arrive(qc, 5);
    read.give([event(3)]);
    await refetch;

    // Stale, so the screen that shows it next asks for what is after event 3.
    await vi.waitFor(() => expect(stale(qc, key)).toBe(true));
    expect(seqs(qc)).toEqual([1, 2, 3]);
    expect(asked()).toHaveLength(1);
  });
});

describe("a change announced while a fetch is in flight", () => {
  const empty: Inbox = { approvals: [], review_runs: [], attention: [], tools_changed: [] };
  /// Shows the Inbox as the app's frame does.
  const watchInbox = (qc: Client) =>
    new QueryObserver(qc, { queryKey: keys.inbox, queryFn: commands.inbox }).subscribe(() => {});
  const loads = () => backend.callsTo("inbox").length;
  /// The first `inbox` call is answered by the test, every later one with `then`.
  function heldFirst(then: Inbox) {
    const read = later<Inbox>();
    backend.on("inbox", () => (loads() === 1 ? read.answer : then));
    return read;
  }

  it("loads the Inbox again when a run changed during its first load", async () => {
    const qc = createQueryClient();
    const read = heldFirst(sample.inbox);
    const unwatch = watchInbox(qc);
    await vi.waitFor(() => expect(loads()).toBe(1));

    // The first load read an empty Inbox; a run fails and waits for a review before it answers.
    applyRuntimeEvent(qc, { type: "run_updated", run: { ...sample.run, status: "failed" } });
    read.give(empty);

    await vi.waitFor(() => expect(qc.getQueryData(keys.inbox)).toEqual(sample.inbox));
    expect(loads()).toBe(2);
    unwatch();
  });

  it("asks once more after a first load, however many changes it missed", async () => {
    const qc = createQueryClient();
    const read = heldFirst(sample.inbox);
    const unwatch = watchInbox(qc);
    await vi.waitFor(() => expect(loads()).toBe(1));

    applyRuntimeEvent(qc, { type: "approval_requested", approval: sample.approval });
    applyRuntimeEvent(qc, { type: "approval_decided", approval: sample.approval });
    invalidate(qc);
    read.give(empty);

    await vi.waitFor(() => expect(qc.getQueryData(keys.inbox)).toEqual(sample.inbox));
    expect(stale(qc, keys.inbox)).toBe(false);
    expect(loads()).toBe(2);
    unwatch();
  });

  it("asks again after a first load that failed", async () => {
    const qc = createQueryClient();
    const read = heldFirst(sample.inbox);
    const unwatch = watchInbox(qc);
    await vi.waitFor(() => expect(loads()).toBe(1));

    invalidate(qc, keys.inbox);
    read.fail({ kind: "internal", message: "database is locked" });

    await vi.waitFor(() => expect(qc.getQueryData(keys.inbox)).toEqual(sample.inbox));
    expect(loads()).toBe(2);
    unwatch();
  });

  it("restarts a refetch of a list on screen and asks no third time", async () => {
    const qc = createQueryClient();
    qc.setQueryData(keys.inbox, empty);
    const unwatch = watchInbox(qc);
    const read = heldFirst(sample.inbox);
    void qc.refetchQueries({ queryKey: keys.inbox });
    await vi.waitFor(() => expect(loads()).toBe(1));

    // TanStack drops the running refetch and starts one that reads after the change.
    invalidate(qc, keys.inbox);
    await vi.waitFor(() => expect(qc.getQueryData(keys.inbox)).toEqual(sample.inbox));
    read.give(empty);
    await vi.waitFor(() => expect(qc.isFetching()).toBe(0));

    expect(qc.getQueryData(keys.inbox)).toEqual(sample.inbox);
    expect(stale(qc, keys.inbox)).toBe(false);
    expect(loads()).toBe(2);
    unwatch();
  });

  it("loads the Dots list again when a dot changed during its first load", async () => {
    const qc = createQueryClient();
    const renamed = { ...sample.overview, dot: { ...sample.dot, name: "renamed" } };
    const read = later<DotOverview[]>();
    const asked = () => backend.callsTo("dot_overview").length;
    backend.on("dot_overview", () => (asked() === 1 ? read.answer : [renamed]));
    const unwatch = new QueryObserver(qc, {
      queryKey: keys.overview,
      queryFn: commands.dotOverview,
    }).subscribe(() => {});
    await vi.waitFor(() => expect(asked()).toBe(1));

    // The list read the dot before it was saved under a new name. No runtime event says so.
    invalidateDots(qc);
    read.give([sample.overview]);

    await vi.waitFor(() => expect(qc.getQueryData(keys.overview)).toEqual([renamed]));
    expect(asked()).toBe(2);
    unwatch();
  });

  it("leaves a query that is removed during its first load alone", async () => {
    const qc = createQueryClient();
    const read = heldFirst(sample.inbox);
    const load = qc.prefetchQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
    await vi.waitFor(() => expect(loads()).toBe(1));

    invalidate(qc, keys.inbox);
    qc.removeQueries({ queryKey: keys.inbox });
    read.give(empty);
    await load;

    expect(qc.getQueryState(keys.inbox)).toBeUndefined();
    expect(loads()).toBe(1);
  });
});
