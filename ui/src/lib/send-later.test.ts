import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { localDayTime } from "@/lib/format";
import { sendsIn, sentLaterLines, waitingText } from "@/lib/send-later";
import { sample } from "@/test/fixtures";

const now = Date.parse("2026-11-04T09:00:00.000Z");
const due = (ms: number) => new Date(now + ms).toISOString();
const MIN = 60_000;
const HOUR = 60 * MIN;
const DAY = 24 * HOUR;

describe("sendsIn", () => {
  it("shows the two largest units, minutes rounded up", () => {
    expect(sendsIn(due(2 * HOUR + 13 * MIN + 10_000), now)).toBe("Sends in 2h 14m");
    expect(sendsIn(due(60 * MIN), now)).toBe("Sends in 1h");
    expect(sendsIn(due(3 * DAY + 4 * HOUR), now)).toBe("Sends in 3d 4h");
    expect(sendsIn(due(20_000), now)).toBe("Sends in 1m");
    expect(sendsIn(due(DAY + 5 * MIN), now)).toBe("Sends in 1d 5m");
  });

  it("says it is sending once it is due", () => {
    expect(sendsIn(due(0), now)).toBe("Sending…");
    expect(sendsIn(due(-5_000), now)).toBe("Sending…");
  });
});

describe("waitingText", () => {
  it("says what a due message waits for", () => {
    expect(waitingText("busy", "nightly")).toBe("Due · waiting for nightly to finish this topic");
    expect(waitingText("approval", "nightly")).toBe(
      "Due · waiting for your approval in this topic",
    );
    expect(waitingText("usage_limit", "nightly")).toBe(
      "Due · waiting for the usage limit to reset",
    );
  });
});

describe("sentLaterLines", () => {
  const chat = (send_later?: unknown): Run => ({
    ...sample.run,
    trigger: "chat",
    queued_at: "2026-11-05T09:10:00.000Z",
    payload: { message: "Ship it", typed: true, ...(send_later ? { send_later } : {}) },
  });
  const note = (late: boolean, new_topic: boolean) => ({
    id: "01J0SCHED",
    due_at: "2026-11-05T09:00:00.000Z",
    late,
    new_topic,
  });

  it("says whether it went late or as scheduled", () => {
    const run = chat(note(true, false));
    const when = localDayTime("2026-11-05T09:00:00.000Z", Date.parse(run.queued_at));
    expect(sentLaterLines(run)).toEqual([`Sent late (was due ${when})`]);
    expect(sentLaterLines(chat(note(false, false)))).toEqual(["Sent as scheduled"]);
  });

  it("says when it started a new topic", () => {
    expect(sentLaterLines(chat(note(false, true)))).toEqual([
      "Sent as scheduled",
      "Started a new topic: the one it was written in can't be continued.",
    ]);
  });

  it("says nothing for any other run", () => {
    expect(sentLaterLines(chat())).toEqual([]);
    expect(sentLaterLines({ ...chat(note(true, false)), trigger: "schedule" })).toEqual([]);
    expect(sentLaterLines(chat({ id: 1 }))).toEqual([]);
    expect(sentLaterLines({ ...sample.run, payload: null })).toEqual([]);
  });
});
