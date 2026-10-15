import { describe, expect, it } from "vitest";

import type { Schedule, ScheduleRule } from "@/api/types";
import {
  everyRule,
  localDateTimeIn,
  mcpOf,
  parseMcp,
  scheduleError,
  scheduleOf,
  specErrors,
  specOf,
  withKind,
} from "@/lib/dot-rules";
import { blankSpec, sample } from "@/test/fixtures";

const good = { ...blankSpec, name: "nightly", instructions: "Run the tests.", workdir: "C:/dev" };

describe("dot editor rules", () => {
  it("accepts a complete dot", () => {
    expect(specErrors(good, "")).toEqual({});
    expect(specErrors(specOf(sample.dot), JSON.stringify(sample.dot.mcp_servers))).toEqual({});
  });

  it("names what a new dot still lacks", () => {
    expect(Object.keys(specErrors(blankSpec, "")).sort()).toEqual([
      "instructions",
      "name",
      "workdir",
    ]);
  });

  it("checks the name the way the core does", () => {
    for (const name of ["", "has space", "ünï", "x".repeat(65)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeDefined();
    }
    for (const name of ["a", "nightly-tests_2", "x".repeat(64)]) {
      expect(specErrors({ ...good, name }, "").name, name).toBeUndefined();
    }
  });

  it("wants an http endpoint for a local dot only", () => {
    const local = { ...good, engine: "local" as const };
    expect(specErrors({ ...local, endpoint_url: null }, "").endpoint_url).toMatch(/needs its/);
    expect(specErrors({ ...local, endpoint_url: "https://x" }, "").endpoint_url).toMatch(/https/);
    expect(specErrors({ ...local, endpoint_url: "http://" }, "").endpoint_url).toBeDefined();
    expect(specErrors({ ...local, endpoint_url: " http://127.0.0.1:1234 " }, "")).toEqual({});
    expect(specErrors({ ...good, endpoint_url: null }, "")).toEqual({});
  });

  it("keeps the limits inside the core's bounds", () => {
    const bad = { max_turns: 0, timeout_secs: 604801, approval_wait_secs: 281 };
    expect(Object.keys(specErrors({ ...good, ...bad }, "")).sort()).toEqual(Object.keys(bad).sort());
    const edge = { max_turns: 1, timeout_secs: 604800, approval_wait_secs: 280 };
    expect(specErrors({ ...good, ...edge }, "")).toEqual({});
    // An emptied number field.
    expect(specErrors({ ...good, max_turns: Number.NaN }, "").max_turns).toMatch(/whole number/);
    expect(specErrors({ ...good, timeout_secs: 1.5 }, "").timeout_secs).toBeDefined();
  });

  it("refuses a rule without a pattern", () => {
    const policy = { ...good.policy, rules: [{ tool: " ", action: "ask" as const }] };
    expect(specErrors({ ...good, policy }, "").policy).toBe("Every rule needs a tool pattern.");
  });

  it("reads the MCP servers field", () => {
    expect(parseMcp("  ")).toEqual({ value: null });
    expect(parseMcp('{"gh": {"type": "http"}}')).toEqual({ value: { gh: { type: "http" } } });
    expect(parseMcp("{nope")).toEqual({ error: "This is not valid JSON." });
    expect(parseMcp("[1]")).toHaveProperty("error");
    expect(parseMcp('"x"')).toHaveProperty("error");
    expect(parseMcp('{"dots": {}}')).toEqual({ error: "The server name 'dots' is reserved." });
    expect(specErrors(good, "{nope").mcp_servers).toBe("This is not valid JSON.");
  });

  it("checks and saves the MCP servers of a Claude dot only", () => {
    const local = { ...good, engine: "local" as const, endpoint_url: "http://127.0.0.1:1234" };
    // A local dot's form hides the field: what it still holds is neither checked nor saved.
    expect(specErrors(local, "{nope")).toEqual({});
    expect(mcpOf(local, '{"gh": {}}')).toEqual({ value: null });
    expect(mcpOf(local, "{nope")).toEqual({ value: null });
    expect(mcpOf(good, '{"gh": {}}')).toEqual({ value: { gh: {} } });
    expect(mcpOf(good, "{nope")).toEqual({ error: "This is not valid JSON." });
  });

  it("takes the editable fields of a stored dot", () => {
    const spec = specOf(sample.dot);
    expect(spec).not.toHaveProperty("id");
    expect(spec).not.toHaveProperty("webhook_token");
    expect(spec.name).toBe(sample.dot.name);
  });
});

describe("schedule rules", () => {
  const [once, everyAnchored, , daily, weekly, cron] = sample.schedules;
  const of = (rule: ScheduleRule, more: Partial<Schedule> = {}): Schedule => ({
    rule,
    catch_up: true,
    until: null,
    ...more,
  });
  // A Wednesday, 10:30 local time.
  const now = new Date(2026, 9, 14, 10, 30);

  it("takes the contract's samples, every kind", () => {
    expect(sample.schedules.map((s) => s.rule.kind)).toEqual([
      "once",
      "every",
      "every",
      "daily",
      "weekly",
      "cron",
    ]);
  });

  it("finds what the core cannot be sent, and leaves the rest to it", () => {
    for (const s of sample.schedules) expect(scheduleError(s), s.rule.kind).toBeUndefined();
    expect(scheduleError(null)).toBeUndefined();
    const every = (minutes: number) => scheduleError(of({ kind: "every", minutes, anchor: null }));
    for (const bad of [Number.NaN, 0, 1.5, 10081]) {
      expect(every(bad), String(bad)).toMatch(/whole minutes/);
    }
    expect(every(1)).toBeUndefined();
    expect(every(10080)).toBeUndefined();
    expect(scheduleError(of({ kind: "daily", time: "" }))).toBe("Pick a time.");
    expect(scheduleError(of({ kind: "weekly", days: ["mon"], time: "" }))).toBe("Pick a time.");
    expect(scheduleError(of({ kind: "once", at: "" }))).toBe("Pick the date and time of the run.");
    expect(scheduleError(of({ kind: "cron", expr: "  " }))).toBe("Write a cron expression.");
    const until = (u: Schedule["until"]) => scheduleError({ ...daily, until: u });
    for (const count of [Number.NaN, 0, 2.5]) {
      expect(until({ count }), String(count)).toMatch(/whole number of runs/);
    }
    expect(until({ count: 1 })).toBeUndefined();
    expect(until({ date: "" })).toBe("Pick the date to stop after.");
    // The core answers for these, through the preview.
    expect(scheduleError(of({ kind: "weekly", days: [], time: "09:00" }))).toBeUndefined();
    expect(scheduleError(of({ kind: "cron", expr: "every day" }))).toBeUndefined();
    // And the draft's checks name it.
    expect(specErrors({ ...good, schedule: of({ kind: "daily", time: "" }) }, "").schedule).toBe(
      "Pick a time.",
    );
  });

  it("trims a cron expression and keeps every other kind as it is", () => {
    expect(scheduleOf(of({ kind: "cron", expr: " 0 0 2 * * * " }))).toEqual(
      of({ kind: "cron", expr: "0 0 2 * * *" }),
    );
    for (const s of sample.schedules) expect(scheduleOf(s)).toEqual(s);
    // An emptied cron field stays a cron schedule: the editor says what is missing.
    expect(scheduleOf(of({ kind: "cron", expr: " " }))).toEqual(of({ kind: "cron", expr: "" }));
    expect(scheduleOf(null)).toBeNull();
  });

  it("writes a local date and time some days ahead", () => {
    expect(localDateTimeIn(1, "09:00", now)).toBe("2026-10-15T09:00");
    expect(localDateTimeIn(30, "23:59", now)).toBe("2026-11-13T23:59");
  });

  it("starts each kind with catch-up on and no stop, carrying the time over", () => {
    expect(withKind(null, "daily", null, now)).toEqual(of({ kind: "daily", time: "09:00" }));
    expect(withKind(null, "weekly", null, now)).toEqual(
      of({ kind: "weekly", days: ["mon"], time: "09:00" }),
    );
    expect(withKind(null, "every", null, now)).toEqual(
      of({ kind: "every", minutes: 60, anchor: null }),
    );
    expect(withKind(null, "cron", null, now)).toEqual(of({ kind: "cron", expr: "" }));
    expect(withKind(null, "once", null, now)).toEqual(
      of({ kind: "once", at: "2026-10-15T09:00" }),
    );
    expect(withKind(daily, "none", null, now)).toBeNull();
    expect(withKind(daily, "daily", null, now)).toBe(daily);
    // The time, the catch-up and the stop go along.
    const quiet = { ...weekly, catch_up: false, until: { count: 3 } };
    expect(withKind(quiet, "daily", null, now)).toEqual(
      of({ kind: "daily", time: "07:30" }, { catch_up: false, until: { count: 3 } }),
    );
    // A once has no stop, and always catches up.
    expect(withKind(quiet, "once", null, now)).toEqual(
      of({ kind: "once", at: "2026-10-15T07:30" }),
    );
    expect(withKind(once, "daily", null, now)).toEqual(of({ kind: "daily", time: "09:00" }));
  });

  it("gives back the stored rule when its kind is picked again", () => {
    const away = withKind(cron, "daily", cron, now);
    expect(withKind(away, "cron", cron, now)).toEqual(cron);
    const viaOnce = withKind(everyAnchored, "once", everyAnchored, now);
    expect(withKind(viaOnce, "every", everyAnchored, now)).toEqual(everyAnchored);
  });

  it("keeps the stored anchor of an every rule only while its minutes stay", () => {
    expect(everyRule(90, everyAnchored)).toEqual(everyAnchored.rule);
    expect(everyRule(60, everyAnchored)).toEqual({ kind: "every", minutes: 60, anchor: null });
    expect(everyRule(90, daily)).toEqual({ kind: "every", minutes: 90, anchor: null });
    expect(everyRule(90, null)).toEqual({ kind: "every", minutes: 90, anchor: null });
  });
});
