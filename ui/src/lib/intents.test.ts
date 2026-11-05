import { describe, expect, it } from "vitest";

import type { Intent } from "@/api/types";
import { addDays, groupIntents, groupOf, localDate } from "@/lib/intents";

const TODAY = "2026-11-05";

function goal(over: Partial<Intent>): Intent {
  return {
    id: over.id ?? "g",
    org_id: null,
    workspace_id: "default",
    text: "A goal",
    due_date: null,
    urgent: false,
    dot_id: null,
    created_at: "2026-11-01T09:00:00.000Z",
    done_at: null,
    ...over,
  };
}

describe("grouping goals by how soon they matter", () => {
  it("puts each date in its group, with today as given", () => {
    const at = (due_date: string | null) => groupOf(goal({ due_date }), TODAY);
    expect(at("2026-11-04")).toBe("overdue");
    expect(at("2025-01-01")).toBe("overdue");
    expect(at(TODAY)).toBe("today");
    // Tomorrow to day 7 are this week; day 8 is later.
    expect(at("2026-11-06")).toBe("week");
    expect(at("2026-11-12")).toBe("week");
    expect(at("2026-11-13")).toBe("later");
    expect(at(null)).toBe("none");
  });

  it("puts an urgent goal under Now, whatever its date", () => {
    expect(groupOf(goal({ urgent: true, due_date: "2026-11-04" }), TODAY)).toBe("now");
    expect(groupOf(goal({ urgent: true, due_date: "2027-01-01" }), TODAY)).toBe("now");
    expect(groupOf(goal({ urgent: true }), TODAY)).toBe("now");
  });

  it("crosses month and year ends", () => {
    expect(addDays("2026-12-29", 7)).toBe("2027-01-05");
    expect(addDays("2026-02-28", 1)).toBe("2026-03-01");
    expect(groupOf(goal({ due_date: "2027-01-05" }), "2026-12-29")).toBe("week");
    expect(groupOf(goal({ due_date: "2027-01-06" }), "2026-12-29")).toBe("later");
  });

  it("reads today in local time", () => {
    // 23:30 local on the 5th is the 5th, whatever UTC says.
    expect(localDate(new Date(2026, 10, 5, 23, 30))).toBe("2026-11-05");
    expect(localDate(new Date(2026, 0, 2, 0, 5))).toBe("2026-01-02");
  });

  it("lists the groups in order, leaves out empty ones, and keeps done goals apart", () => {
    const goals = [
      goal({ id: "none" }),
      goal({ id: "later", due_date: "2026-12-01" }),
      goal({ id: "week2", due_date: "2026-11-09" }),
      goal({ id: "week1", due_date: "2026-11-07" }),
      goal({ id: "overdue", due_date: "2026-11-01" }),
      goal({ id: "now", urgent: true }),
      goal({ id: "old-done", done_at: "2026-11-02T09:00:00.000Z", urgent: true }),
      goal({ id: "new-done", done_at: "2026-11-04T09:00:00.000Z" }),
    ];
    const { groups, done } = groupIntents(goals, TODAY);
    expect(groups.map((g) => [g.label, g.intents.map((i) => i.id)])).toEqual([
      ["Now", ["now"]],
      ["Overdue", ["overdue"]],
      ["This week", ["week1", "week2"]],
      ["Later", ["later"]],
      ["No date", ["none"]],
    ]);
    // Newest done first.
    expect(done.map((i) => i.id)).toEqual(["new-done", "old-done"]);
  });
});
