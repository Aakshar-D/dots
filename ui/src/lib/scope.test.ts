import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { sample } from "@/test/fixtures";

import { inScope, waitingByAccount } from "./scope";

const row = (org_id: string | null, needs_you: number): DotOverview => ({
  ...sample.overview,
  dot: { ...sample.dot, org_id },
  needs_you,
});

describe("inScope", () => {
  it("matches Personal to the dots with no org", () => {
    const scope = { account: "personal", workspace: "default" };
    expect(inScope({ org_id: null }, scope)).toBe(true);
    expect(inScope({ org_id: "acme-eng" }, scope)).toBe(false);
  });

  it("matches an org to its own dots", () => {
    const scope = { account: "acme-eng", workspace: "default" };
    expect(inScope({ org_id: "acme-eng" }, scope)).toBe(true);
    expect(inScope({ org_id: "design" }, scope)).toBe(false);
    expect(inScope({ org_id: null }, scope)).toBe(false);
  });
});

describe("waitingByAccount", () => {
  it("sums what waits per account", () => {
    const waiting = waitingByAccount([row(null, 1), row("acme-eng", 2), row("acme-eng", 3), row(null, 0)]);
    expect(waiting.get("personal")).toBe(1);
    expect(waiting.get("acme-eng")).toBe(5);
    expect(waiting.get("design")).toBeUndefined();
  });
});
