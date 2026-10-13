import { describe, expect, it } from "vitest";

import { duration, localDateTime, relativeTime } from "./format";

const at = (iso: string) => Date.parse(iso);

describe("format", () => {
  it("writes a duration in its two largest units", () => {
    const start = "2026-10-12T10:00:00.000Z";
    expect(duration(start, "2026-10-12T10:00:45.900Z")).toBe("45s");
    expect(duration(start, "2026-10-12T10:03:05.000Z")).toBe("3m 05s");
    expect(duration(start, "2026-10-12T12:07:59.000Z")).toBe("2h 07m");
    // Still running: up to now.
    expect(duration(start, null, at("2026-10-12T10:00:10.000Z"))).toBe("10s");
    expect(duration(null, null)).toBe("");
    expect(duration(start, "2026-10-12T09:00:00.000Z")).toBe("");
  });

  it("writes a time relative to now, behind or ahead", () => {
    const now = at("2026-10-12T10:00:00.000Z");
    expect(relativeTime("2026-10-12T09:59:40.000Z", now)).toBe("just now");
    expect(relativeTime("2026-10-12T09:55:00.000Z", now)).toBe("5 min ago");
    expect(relativeTime("2026-10-12T07:00:00.000Z", now)).toBe("3 h ago");
    expect(relativeTime("2026-10-10T10:00:00.000Z", now)).toBe("2 d ago");
    expect(relativeTime("2026-10-12T10:05:00.000Z", now)).toBe("in 5 min");
    expect(relativeTime(null, now)).toBe("");
    expect(relativeTime("not a time", now)).toBe("");
  });

  it("keeps text that is not a timestamp", () => {
    expect(localDateTime(null)).toBe("");
    expect(localDateTime("soon")).toBe("soon");
    expect(localDateTime("2026-10-12T10:00:00.000Z")).not.toBe("");
  });
});
