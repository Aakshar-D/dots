import { describe, expect, it } from "vitest";

import type { DotOverview } from "@/api/types";
import { sample } from "@/test/fixtures";

import { accountOf, inScope, waitingByAccount } from "./scope";

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

describe("inScope, workspace", () => {
  it("inScope compares workspace", () => {
    const scope = { account: "personal", workspace: "ops" };
    expect(inScope({ org_id: null, workspace_id: "ops" }, scope)).toBe(true);
    expect(inScope({ org_id: null, workspace_id: "default" }, scope)).toBe(false);
    const acme = { account: "acme-eng", workspace: "default" };
    expect(inScope({ org_id: "acme-eng", workspace_id: "ops" }, acme)).toBe(false);
    expect(inScope({ org_id: "acme-eng", workspace_id: "default" }, acme)).toBe(true);
  });
});

describe("a dot whose account was removed", () => {
  const joined = [{ ...sample.orgs[0], id: "acme-eng" }];

  it("belongs to Personal once the joined accounts are known", () => {
    expect(accountOf({ org_id: "gone" }, joined)).toBe("personal");
    expect(accountOf({ org_id: "acme-eng" }, joined)).toBe("acme-eng");
    expect(accountOf({ org_id: null }, joined)).toBe("personal");
    expect(inScope({ org_id: "gone" }, { account: "personal", workspace: "default" }, joined)).toBe(
      true,
    );
    expect(inScope({ org_id: "gone" }, { account: "acme-eng", workspace: "default" }, joined)).toBe(
      false,
    );
  });

  it("is listed in each Personal workspace, whatever workspace it was in", () => {
    for (const workspace of ["default", "ops"]) {
      const personal = { account: "personal", workspace };
      expect(inScope({ org_id: "gone", workspace_id: "payments" }, personal, joined)).toBe(true);
      expect(inScope({ org_id: "gone", workspace_id: "default" }, personal, joined)).toBe(true);
    }
    // A Personal dot still keeps to its workspace.
    const ops = { account: "personal", workspace: "ops" };
    expect(inScope({ org_id: null, workspace_id: "default" }, ops, joined)).toBe(false);
  });

  it("is not guessed while the joined accounts load", () => {
    expect(accountOf({ org_id: "gone" }, undefined)).toBeUndefined();
    expect(accountOf({ org_id: null }, undefined)).toBe("personal");
    expect(inScope({ org_id: "gone" }, { account: "personal", workspace: "default" })).toBe(false);
  });

  it("counts what waits in it under Personal", () => {
    const waiting = waitingByAccount([row(null, 1), row("gone", 2), row("acme-eng", 3)], joined);
    expect(waiting.get("personal")).toBe(3);
    expect(waiting.get("gone")).toBeUndefined();
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
