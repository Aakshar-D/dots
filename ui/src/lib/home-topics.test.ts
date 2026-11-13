import { describe, expect, it } from "vitest";

import type { Run, TopicInfo } from "@/api/types";
import { localTime } from "@/lib/format";
import {
  archivedTopics,
  conversation,
  currentTopic,
  defaultTarget,
  lastRun,
  liveTopics,
  olderTopics,
  oneLine,
  topicOf,
  topicsOf,
  topicTitle,
  withInfo,
} from "@/lib/home-topics";
import { sample } from "@/test/fixtures";

/// A root run queued at `at` (minutes after 10:00 on 9 October); a chat run unless `over` says.
const root = (id: string, at: number, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: id,
  parent_run_id: null,
  trigger: "chat",
  payload: { message: `message of ${id}` },
  queued_at: new Date(Date.UTC(2026, 9, 9, 10, at)).toISOString(),
  ...over,
});
/// A run that continued `parent`, queued at `at`.
const reply = (id: string, parent: Run, at: number): Run =>
  root(id, at, { root_run_id: parent.root_run_id, parent_run_id: parent.id });
const ids = (topics: { rootId: string }[]) => topics.map((t) => t.rootId);

describe("topics", () => {
  it("groups runs into lines by their root, ordered by the roots' queue times", () => {
    const a = root("a", 0);
    const cron = root("cron", 5, { trigger: "schedule", payload: null });
    const b = root("b", 10);
    const topics = topicsOf([b, reply("a2", a, 20), cron, a, reply("cron1", cron, 30)]);
    expect(ids(topics)).toEqual(["a", "cron", "b"]);
    expect(topics.map((t) => t.kind)).toEqual(["chat", "run", "chat"]);
    expect(topics[0].runs.map((r) => r.id)).toEqual(["a", "a2"]);
    expect(topics[1].runs.map((r) => r.id)).toEqual(["cron", "cron1"]);
  });

  it("takes the oldest loaded run as the root when the root is older than what was loaded", () => {
    const gone = root("gone", 0, { trigger: "schedule" });
    const topic = topicOf([reply("r2", gone, 20), reply("r1", gone, 10)]);
    expect(topic.rootId).toBe("gone");
    expect(topic.root.id).toBe("r1");
    // The oldest run left is a reply, a chat run: the line reads as a chat.
    expect(topic.kind).toBe("chat");
  });

  it("makes the newest chat topic current, with the run topics after it", () => {
    const old = root("old", 0);
    const before = root("before", 5, { trigger: "schedule" });
    const current = root("current", 10);
    const after = root("after", 15, { trigger: "webhook" });
    const manual = root("manual", 20, { trigger: "manual" });
    const topics = topicsOf([old, before, current, after, manual, reply("old2", old, 30)]);
    expect(currentTopic(topics)?.rootId).toBe("current");
    expect(ids(conversation(topics))).toEqual(["current", "after", "manual"]);
    // History: everything else, newest first.
    expect(ids(olderTopics(topics, conversation(topics)))).toEqual(["before", "old"]);
  });

  it("shows the last 20 run topics of a sol that has no chat topic", () => {
    const runs = Array.from({ length: 25 }, (_, i) =>
      root(`r${String(i).padStart(2, "0")}`, i, { trigger: "schedule" }),
    );
    const topics = topicsOf(runs);
    expect(currentTopic(topics)).toBeUndefined();
    const shown = conversation(topics);
    expect(shown).toHaveLength(20);
    expect(shown[0].rootId).toBe("r05");
    expect(shown.at(-1)!.rootId).toBe("r24");
    expect(ids(olderTopics(topics, shown))).toEqual(["r04", "r03", "r02", "r01", "r00"]);
  });

  it("lists 50 older topics at most", () => {
    const runs = Array.from({ length: 60 }, (_, i) => root(`c${String(i).padStart(2, "0")}`, i));
    const topics = topicsOf(runs);
    const older = olderTopics(topics, conversation(topics));
    expect(older).toHaveLength(50);
    expect(older[0].rootId).toBe("c58");
  });

  it("replies by default to the current chat topic, whatever ran after it", () => {
    const current = root("current", 0);
    const cron = root("cron", 10, { trigger: "schedule" });
    expect(defaultTarget(topicsOf([current, cron]))?.rootId).toBe("current");
    expect(defaultTarget(topicsOf([current, cron, reply("c2", current, 20)]))?.rootId).toBe(
      "current",
    );
    // Only run topics: a message starts a new chat topic.
    expect(defaultTarget(topicsOf([cron]))).toBeUndefined();
    expect(defaultTarget([])).toBeUndefined();
  });

  it("shows the current topic with the last 20 run topics after it; the rest go to History", () => {
    const current = root("current", 0);
    const after = Array.from({ length: 25 }, (_, i) =>
      root(`s${String(i).padStart(2, "0")}`, i + 1, { trigger: "schedule" }),
    );
    const topics = topicsOf([current, ...after]);
    const shown = conversation(topics);
    expect(shown).toHaveLength(21);
    expect(shown[0].rootId).toBe("current");
    expect(shown[1].rootId).toBe("s05");
    expect(shown.at(-1)!.rootId).toBe("s24");
    expect(ids(olderTopics(topics, shown))).toEqual(["s04", "s03", "s02", "s01", "s00"]);
  });

  it("names a chat topic by its first line and a run topic by its trigger and time", () => {
    const named = (over: Partial<Run>) => topicTitle(topicOf([root("a", 0, over)]));
    expect(named({ payload: { message: "  Fix the bug\nand add a test" } })).toBe("Fix the bug");
    expect(named({ payload: { message: "x".repeat(100) } })).toBe(`${"x".repeat(60)}…`);
    expect(named({ payload: { message: "" } })).toBe("Chat");
    const at = root("a", 0).queued_at;
    expect(named({ trigger: "schedule", payload: null })).toBe(`Scheduled · ${localTime(at)}`);
    expect(named({ trigger: "webhook", payload: null })).toBe(`Webhook · ${localTime(at)}`);
    expect(named({ trigger: "manual", payload: null })).toBe(`Run now · ${localTime(at)}`);
  });

  it("cuts text to its first line", () => {
    expect(oneLine(" one\ntwo ")).toBe("one");
    expect(oneLine(null)).toBe("");
    expect(oneLine("abcdef", 3)).toBe("abc…");
  });
});

describe("topic rows", () => {
  const info = (root_run_id: string, over: Partial<TopicInfo> = {}): TopicInfo => ({
    root_run_id,
    dot_id: sample.run.dot_id,
    title: null,
    archived_at: null,
    ...over,
  });

  it("names a topic by the title the person gave it", () => {
    const [a, b] = withInfo(topicsOf([root("a", 0), root("b", 5)]), [
      info("a", { title: "Trip plans" }),
    ]);
    expect(topicTitle(a)).toBe("Trip plans");
    expect(topicTitle(b)).toBe("message of b");
  });

  it("leaves archived topics out of the conversation and History, and lists them apart", () => {
    const topics = withInfo(topicsOf([root("a", 0), root("b", 5), root("c", 10)]), [
      info("a", { archived_at: "2026-10-10T00:00:00Z" }),
      info("c", { archived_at: "2026-10-10T00:00:00Z" }),
    ]);
    const live = liveTopics(topics);
    const shown = conversation(live);
    expect(ids(shown)).toEqual(["b"]);
    expect(ids(olderTopics(live, shown))).toEqual([]);
    expect(ids(archivedTopics(topics))).toEqual(["c", "a"]);
  });
});

describe("a topic's line", () => {
  it("reads along the parent links, so runs moved in from older times come last", () => {
    const b = root("b", 10);
    const b2 = reply("b2", b, 11);
    // Moved in: older than b's runs, hung after b2.
    const a2 = root("a2", 1, { root_run_id: "b", parent_run_id: "b2" });
    const a3 = root("a3", 2, { root_run_id: "b", parent_run_id: "a2" });
    const topic = topicOf([a3, b, a2, b2]);
    expect(topic.runs.map((r) => r.id)).toEqual(["b", "b2", "a2", "a3"]);
    expect(topic.root.id).toBe("b");
    expect(lastRun(topic)?.id).toBe("a3");
  });
});

describe("lastRun", () => {
  it("skips a dismissed last run, so a reply continues the live line", () => {
    const a = root("a", 0);
    const retried = reply("a2", a, 10);
    const dismissed = reply("a3", a, 20);
    const topic = topicOf([a, retried, { ...dismissed, dismissed_at: "2026-10-09T10:21:00Z", delivered_at: null }]);
    expect(lastRun(topic)?.id).toBe("a2");
  });

  it("still picks a dismissed last run that was delivered", () => {
    const a = root("a", 0);
    const dismissed = {
      ...reply("a2", a, 10),
      dismissed_at: "2026-10-09T10:21:00Z",
      delivered_at: "2026-10-09T10:11:00Z",
    };
    expect(lastRun(topicOf([a, dismissed]))?.id).toBe("a2");
  });

  it("is the last run when none is dismissed", () => {
    expect(lastRun(topicOf([root("b", 0), reply("b2", root("b", 0), 5)]))?.id).toBe("b2");
  });

  it("is none when every run was dismissed and never delivered", () => {
    const a = { ...root("a", 0), dismissed_at: "2026-10-09T10:21:00Z", delivered_at: null };
    expect(lastRun(topicOf([a]))).toBeUndefined();
  });
});
