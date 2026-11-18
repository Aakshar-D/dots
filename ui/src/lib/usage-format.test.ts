import { describe, expect, it } from "vitest";

import { localTime } from "@/lib/format";
import {
  count,
  formatCost,
  formatDuration,
  formatTokens,
  pct,
  projectNames,
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
    expect(formatTokens(999_900_000)).toBe("999.9M");
    expect(formatTokens(5_732_100_000)).toBe("5.7B");
    expect(formatTokens(12_000_000_000)).toBe("12B");
  });

  it("count one thing in the singular", () => {
    expect(count(1, "message")).toBe("1 message");
    expect(count(0, "message")).toBe("0 messages");
    expect(count(2, "session")).toBe("2 sessions");
  });

  it("name a project by its folder, with its parent when two share the name", () => {
    expect(
      projectNames([
        { cwd: String.raw`C:\work\app\src`, name: "src" },
        { cwd: "/home/me/site/src/", name: "src" },
        { cwd: "C:/work/dots", name: "dots" },
        { cwd: null, name: "Sage (all sols)" },
        { cwd: "", name: "(unknown folder)" },
      ]),
    ).toEqual(["app/src", "site/src", "dots", "Sage (all sols)", "(unknown folder)"]);
  });

  it("say a run time in seconds, minutes or hours", () => {
    expect(formatDuration(45)).toBe("45s");
    expect(formatDuration(724)).toBe("12m 04s");
    expect(formatDuration(11_520)).toBe("3h 12m");
  });

  it("round a cost to cents and a share to at most 100%", () => {
    expect(formatCost(1.3249)).toBe("$1.32");
    expect(formatCost(18.4)).toBe("$18.40");
    // Self-QA: a Haiku run's $0.004 read as $0.00, as if it had cost nothing.
    expect(formatCost(0.0038)).toBe("<$0.01");
    expect(formatCost(0)).toBe("$0.00");
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
