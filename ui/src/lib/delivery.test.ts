import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { deliveryLine } from "@/lib/delivery";
import { sample } from "@/test/fixtures";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const MIN = 60_000;
const waiting = (minutes: number, reason: Run["wait_reason"]): Run => ({
  ...sample.run,
  status: "queued",
  delivery: "waiting",
  wait_reason: reason,
  queued_at: new Date(NOW - minutes * MIN).toISOString(),
});

describe("deliveryLine", () => {
  it("says nothing before 5 whole minutes", () => {
    expect(deliveryLine(waiting(4, "busy"), NOW)).toBeNull();
  });

  it("says why a message has waited 5 minutes, for each reason", () => {
    const texts = {
      busy: "other runs are busy",
      usage_hold: "held for the usage limit",
      updating: "Sage is updating",
      dot_busy: "this sol is busy with another run",
      queued: "next in line",
    } as const;
    for (const [reason, text] of Object.entries(texts)) {
      expect(deliveryLine(waiting(5, reason as keyof typeof texts), NOW)).toEqual({
        kind: "waiting",
        text: `Waiting 5 min · ${text}`,
      });
    }
  });

  it("counts the minutes", () => {
    expect(deliveryLine(waiting(12, "queued"), NOW)?.text).toBe("Waiting 12 min · next in line");
  });

  it("says a marked message did not get there, and why", () => {
    const run: Run = {
      ...sample.run,
      delivery: "not_delivered",
      undelivered_at: "2026-11-02T11:00:00Z",
      undelivered_reason: "claude CLI not found",
    };
    expect(deliveryLine(run, NOW)).toEqual({
      kind: "not_delivered",
      text: "Not delivered: claude CLI not found",
    });
  });

  it("says plain Not delivered when no reason is known", () => {
    for (const reason of [null, "", "  "]) {
      const run: Run = {
        ...sample.run,
        delivery: "not_delivered",
        undelivered_at: "2026-11-02T11:00:00Z",
        undelivered_reason: reason,
      };
      expect(deliveryLine(run, NOW)).toEqual({ kind: "not_delivered", text: "Not delivered" });
    }
  });

  it("says only Not delivered once it is dismissed", () => {
    const run: Run = {
      ...sample.run,
      delivery: "not_delivered",
      undelivered_reason: "claude CLI not found",
      dismissed_at: "2026-11-02T11:30:00Z",
    };
    expect(deliveryLine(run, NOW)).toEqual({ kind: "dismissed", text: "Not delivered" });
  });

  it("says nothing for a delivered message", () => {
    expect(deliveryLine({ ...sample.run, delivery: "delivered" }, NOW)).toBeNull();
  });
});
