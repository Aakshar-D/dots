import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { chatThreads } from "@/lib/chat-threads";
import { sample } from "@/test/fixtures";

const run = (id: string, at: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: id,
  parent_run_id: null,
  trigger: "chat",
  payload: { message: `message of ${id}` },
  queued_at: `2026-10-09T${at}:00.000Z`,
  ...over,
});

describe("chat threads", () => {
  it("groups a dot's runs into chat threads, the most recently active first", () => {
    const threads = chatThreads([
      run("a", "10:00"),
      run("b", "11:00"),
      run("a2", "12:00", { root_run_id: "a", parent_run_id: "a", status: "running" }),
      run("a1", "10:30", { root_run_id: "a", parent_run_id: "a", trigger: "resume" }),
      run("cron", "13:00", { trigger: "schedule", payload: null }),
    ]);
    expect(threads.map((t) => [t.rootId, t.title, t.latest.id])).toEqual([
      ["a", "message of a", "a2"],
      ["b", "message of b", "b"],
    ]);
  });

  it("titles a thread by the first line of its first message", () => {
    const title = (message: unknown) => chatThreads([run("a", "10:00", { payload: { message } })])[0].title;
    expect(title("  Fix the bug\nand add a test")).toBe("Fix the bug");
    expect(title("x".repeat(100))).toBe(`${"x".repeat(60)}…`);
    expect(title("")).toBe("Chat");
    expect(title(7)).toBe("Chat");
  });
});
