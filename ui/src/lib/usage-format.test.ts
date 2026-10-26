import { describe, expect, it } from "vitest";

import { localTime } from "@/lib/format";
import {
  formatCost,
  formatDuration,
  formatTokens,
  pct,
  resetTime,
  runSeconds,
} from "@/lib/usage-format";

describe("the usage numbers", () => {
  it("shorten tokens to k and M", () => {
    expect(formatTokens(950)).toBe("950");
    expect(formatTokens(1540)).toBe("1.5k");
    expect(formatTokens(12_400)).toBe("12k");
    expect(formatTokens(182_000)).toBe("182k");
    expect(formatTokens(2_100_000)).toBe("2.1M");
  });

  it("say a run time in seconds, minutes or hours", () => {
    expect(formatDuration(45)).toBe("45s");
    expect(formatDuration(724)).toBe("12m 04s");
    expect(formatDuration(11_520)).toBe("3h 12m");
  });

  it("round a cost to cents and a share to at most 100%", () => {
    expect(formatCost(1.3249)).toBe("$1.32");
    expect(formatCost(18.4)).toBe("$18.40");
    expect(pct(0.424)).toBe(42);
    expect(pct(1.4)).toBe(100);
  });

  it("measure a run from start to end, and nothing until it has ended", () => {
    const started_at = "2026-10-26T12:00:00.000Z";
    expect(runSeconds({ started_at, ended_at: "2026-10-26T12:12:04.000Z" })).toBe(724);
    expect(runSeconds({ started_at, ended_at: null })).toBeNull();
    expect(runSeconds({ started_at: null, ended_at: null })).toBeNull();
  });

  it("say a reset by its time today, and with its weekday on another day", () => {
    const now = Date.parse("2026-10-26T12:00:00");
    const today = new Date("2026-10-26T15:10:00").toISOString();
    const thursday = new Date("2026-10-29T09:00:00").toISOString();
    expect(resetTime(today, now)).toBe(localTime(today));
    expect(resetTime(thursday, now).endsWith(` ${localTime(thursday)}`)).toBe(true);
    expect(resetTime(thursday, now)).not.toBe(localTime(thursday));
  });
});
