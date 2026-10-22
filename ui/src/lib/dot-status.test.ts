import { describe, expect, it } from "vitest";

import type { DotOverview, Message, Run } from "@/api/types";
import { dotStatus, lastLine, nextLine, sortDots } from "@/lib/dot-status";
import { localTime } from "@/lib/format";
import { sample } from "@/test/fixtures";

/// An enabled dot named `name` with no schedule and no run.
const row = (name: string, over: Partial<DotOverview> = {}): DotOverview => ({
  dot: { ...sample.dot, id: `dot-${name}`, name, schedule: null, enabled: true },
  active_run: null,
  last_run: null,
  schedule_state: null,
  needs_you: 0,
  unread: 0,
  ...over,
});
const run = (status: Run["status"], over: Partial<Run> = {}): Run => ({
  ...sample.run,
  status,
  ...over,
});
const attention = sample.overview.schedule_state!;
const [openQuestion] = sample.messages;

describe("a dot's status mark", () => {
  it("takes the first that matches: waiting, running, attention, off, idle", () => {
    const question: Message = { ...openQuestion, dot_id: "dot-a" };
    const off = { ...sample.dot, id: "dot-a", name: "a", enabled: false };
    const everything = row("a", {
      dot: off,
      active_run: run("awaiting_approval"),
      schedule_state: attention,
    });
    expect(dotStatus(everything, [])).toBe("waiting");
    expect(dotStatus({ ...everything, active_run: run("running") }, [question])).toBe("waiting");
    expect(dotStatus({ ...everything, active_run: run("running") }, [])).toBe("running");
    expect(dotStatus({ ...everything, active_run: run("queued") }, [])).toBe("running");
    expect(dotStatus({ ...everything, active_run: null }, [])).toBe("attention");
    expect(dotStatus({ ...everything, active_run: null, schedule_state: null }, [])).toBe("off");
    expect(dotStatus(row("a"), [])).toBe("idle");
  });

  it("counts only the dot's own questions that are still open", () => {
    const [, answered, , note] = sample.messages;
    const mine = (m: Message): Message => ({ ...m, dot_id: "dot-a" });
    expect(dotStatus(row("a"), [mine(answered), mine(note)])).toBe("idle");
    expect(dotStatus(row("a"), [{ ...openQuestion, dot_id: "dot-b" }])).toBe("idle");
  });
});

describe("the column", () => {
  it("puts the newest activity first and the dots that never ran after, by name", () => {
    const at = (h: number) => `2026-10-09T${String(h).padStart(2, "0")}:00:00.000Z`;
    const rows = [
      row("zed"),
      row("old", { last_run: run("succeeded", { queued_at: at(8) }) }),
      row("busy", {
        last_run: run("succeeded", { queued_at: at(7) }),
        active_run: run("running", { queued_at: at(11) }),
      }),
      row("alpha"),
      row("new", { last_run: run("failed", { queued_at: at(10) }) }),
    ];
    expect(sortDots(rows).map((r) => r.dot.name)).toEqual(["busy", "new", "old", "alpha", "zed"]);
  });

  it("says the last run's summary, else its error, else its message, on one line", () => {
    const last = (over: Partial<Run>) => lastLine(row("a", { last_run: run("succeeded", over) }));
    expect(last({ summary: "Placed order 42\nand mailed it", error: "x" })).toBe("Placed order 42");
    expect(last({ summary: null, error: "timeout" })).toBe("timeout");
    expect(last({ summary: "  ", error: null, payload: { message: "order the usual" } })).toBe(
      "order the usual",
    );
    expect(last({ summary: null, error: null, payload: null })).toBe("");
    expect(lastLine(row("a"))).toBe("");
  });

  it("says when the dot runs next, else when its next follow-up fires", () => {
    const [pending] = sample.followUps;
    const soon = new Date(Date.now() + 3 * 3_600_000).toISOString();
    const followUp = { ...pending, dot_id: "dot-a", due_at: soon };
    const scheduled = row("a", { schedule_state: attention });
    expect(nextLine(scheduled, [followUp])).toBe(`next ${localTime(attention.next_fire)}`);
    expect(nextLine(row("a"), [followUp])).toBe("Next follow-up in 3 h");
    expect(nextLine(row("a"), [{ ...followUp, dot_id: "dot-b" }])).toBe("");
  });
});
