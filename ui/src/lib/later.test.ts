import { describe, expect, it } from "vitest";

import { localDateTime, localTime, localWeekdayTime } from "@/lib/format";
import { backText, fromPicker, isLater, laterPresets, toPicker } from "@/lib/later";

/// The local date and time of `d`, to read an assertion at a glance.
const local = (d: Date) => [
  d.getFullYear(),
  d.getMonth() + 1,
  d.getDate(),
  d.getHours(),
  d.getMinutes(),
];
const byId = (now: Date) => Object.fromEntries(laterPresets(now).map((p) => [p.id, p.at]));

describe("laterPresets", () => {
  it("laterPresets_at_the_edges", () => {
    // Wednesday 4 November 2026, 10:00.
    const wed = laterPresets(new Date(2026, 10, 4, 10, 0));
    expect(wed.map((p) => p.label)).toEqual([
      "In 1 hour",
      "This evening (6 PM)",
      "Tomorrow morning (9 AM)",
      "Next week (Monday 9 AM)",
    ]);
    expect(wed.map((p) => local(p.at))).toEqual([
      [2026, 11, 4, 11, 0],
      [2026, 11, 4, 18, 0],
      [2026, 11, 5, 9, 0],
      [2026, 11, 9, 9, 0],
    ]);

    // A minute before 6 PM, "This evening" is a minute ahead.
    expect(local(byId(new Date(2026, 10, 4, 17, 59)).evening)).toEqual([2026, 11, 4, 18, 0]);
    // From 6 PM it is not offered: a passed preset is hidden, not rolled to tomorrow.
    for (const now of [new Date(2026, 10, 4, 18, 0), new Date(2026, 10, 4, 18, 30)]) {
      expect(laterPresets(now).map((p) => p.id)).toEqual(["hour", "morning", "next_week"]);
    }

    // On a Monday, next week is the following Monday.
    expect(local(byId(new Date(2026, 10, 9, 8, 0)).next_week)).toEqual([2026, 11, 16, 9, 0]);
    // On a Sunday, it is the next day.
    expect(local(byId(new Date(2026, 10, 8, 12, 0)).next_week)).toEqual([2026, 11, 9, 9, 0]);
    // The last day of the year: tomorrow morning is 1 January.
    expect(local(byId(new Date(2026, 11, 31, 22, 0)).morning)).toEqual([2027, 1, 1, 9, 0]);
    // The last day of a month.
    expect(local(byId(new Date(2026, 10, 30, 7, 0)).morning)).toEqual([2026, 12, 1, 9, 0]);
  });
});

describe("isLater", () => {
  const now = new Date(2026, 10, 4, 10, 0);
  it("takes a time after now and at most 365 days ahead", () => {
    expect(isLater(now, now)).toBe(false);
    expect(isLater(new Date(2026, 10, 4, 9, 59), now)).toBe(false);
    expect(isLater(new Date(2026, 10, 4, 10, 1), now)).toBe(true);
    expect(isLater(new Date(now.getTime() + 365 * 86_400_000), now)).toBe(true);
    expect(isLater(new Date(now.getTime() + 366 * 86_400_000), now)).toBe(false);
  });
});

describe("the date and time picker", () => {
  it("reads and writes local times", () => {
    const at = fromPicker("2026-11-05T09:00");
    expect(at && local(at)).toEqual([2026, 11, 5, 9, 0]);
    expect(fromPicker("")).toBeNull();
    expect(fromPicker("tomorrow")).toBeNull();
    expect(toPicker(new Date(2026, 0, 2, 3, 4))).toBe("2026-01-02T03:04");
  });
});

describe("backText", () => {
  it("says when an item comes back: the time today, the weekday within 6 days, else the date", () => {
    const now = new Date(2026, 10, 4, 10, 0).getTime();
    const today = new Date(2026, 10, 4, 18, 0).toISOString();
    expect(backText(today, now)).toBe(`Back ${localTime(today)}`);
    const monday = new Date(2026, 10, 9, 9, 0).toISOString();
    expect(backText(monday, now)).toBe(`Back ${localWeekdayTime(monday, now)}`);
    expect(backText(monday, now)).not.toBe(`Back ${localTime(monday)}`);
    const far = new Date(2026, 11, 25, 9, 0).toISOString();
    expect(backText(far, now)).toBe(`Back ${localDateTime(far)}`);
  });
});
