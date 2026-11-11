import { describe, expect, it } from "vitest";

import type { Schedule } from "@/api/types";
import { localDateTime } from "@/lib/format";

import { scheduleWords } from "./cards";

const at = (rule: Schedule["rule"], until: Schedule["until"] = null): Schedule => ({
  rule,
  catch_up: false,
  until,
});
const every = (minutes: number) => at({ kind: "every", minutes, anchor: null });

describe("a schedule in words", () => {
  it.each<[number, string]>([
    [1, "Every minute"],
    [15, "Every 15 minutes"],
    [60, "Every hour"],
    [120, "Every 2 hours"],
    [1440, "Every day"],
    [2880, "Every 2 days"],
    [36 * 60, "Every 36 hours"],
    [168 * 60, "Every week"],
    [2 * 10080, "Every 2 weeks"],
    [90, "Every 90 minutes"],
  ])("every %i minutes is %s", (minutes, words) => {
    expect(scheduleWords(every(minutes))).toBe(words);
  });

  it("says the days of a weekly schedule, or that none was chosen", () => {
    expect(scheduleWords(at({ kind: "weekly", days: ["mon", "tue", "wed", "thu", "fri"], time: "09:00" }))).toBe(
      "Weekdays at 09:00",
    );
    expect(scheduleWords(at({ kind: "weekly", days: ["sun", "sat"], time: "10:00" }))).toBe("Weekends at 10:00");
    expect(scheduleWords(at({ kind: "weekly", days: ["thu", "mon"], time: "08:30" }))).toBe(
      "Every Monday and Thursday at 08:30",
    );
    expect(scheduleWords(at({ kind: "weekly", days: [], time: "09:00" }))).toBe("No days chosen");
  });

  it("says a daily and a one-off schedule", () => {
    expect(scheduleWords(at({ kind: "daily", time: "07:00" }))).toBe("Every day at 07:00");
    expect(scheduleWords(at({ kind: "once", at: "2031-05-06T12:00:00Z" }))).toBe(
      `Once, ${localDateTime("2031-05-06T12:00:00Z")}`,
    );
  });

  it("shows a cron schedule as a custom one", () => {
    expect(scheduleWords(at({ kind: "cron", expr: "0 9 * * 1-5" }))).toBe("On a custom schedule (0 9 * * 1-5)");
  });

  it("adds when it stops, the date in the local format", () => {
    expect(scheduleWords(at({ kind: "daily", time: "07:00" }, { count: 1 }))).toBe("Every day at 07:00, 1 time");
    expect(scheduleWords(at({ kind: "daily", time: "07:00" }, { count: 5 }))).toBe("Every day at 07:00, 5 times");
    const until = scheduleWords(at({ kind: "daily", time: "07:00" }, { date: "2031-05-06T12:00" }));
    expect(until).toBe(`Every day at 07:00, until ${localDateTime("2031-05-06T12:00")}`);
    expect(until).not.toContain("2031-05-06T12:00");
  });
});
