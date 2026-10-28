import { describe, expect, it } from "vitest";

import type { OrgSummary, Role } from "@/api/types";
import { sample } from "@/test/fixtures";

import { accessOf, allows, notAllowedText, type Action } from "./access";

const base = sample.orgs[0];
const org = (over: Partial<OrgSummary>): OrgSummary => ({ ...base, ...over }) as OrgSummary;
const ACTIONS: Action[] = ["edit", "run", "answer"];

describe("accessOf", () => {
  it("is Owner in Personal, Legacy for an unjoined or list-less account", () => {
    expect(accessOf(undefined, null)).toEqual({ kind: "owner" });
    expect(accessOf(undefined, "gone")).toEqual({ kind: "legacy" });
    expect(accessOf(org({ membership: "legacy", role: null }), base.id)).toEqual({ kind: "legacy" });
  });

  it("carries the role of an active member and fails closed without one", () => {
    expect(accessOf(org({ membership: "active", role: "viewer" }), base.id)).toEqual({
      kind: "role",
      role: "viewer",
    });
    expect(accessOf(org({ membership: "active", role: null }), base.id)).toEqual({
      kind: "waiting",
    });
    expect(accessOf(org({ membership: "waiting", role: null }), base.id).kind).toBe("waiting");
    expect(accessOf(org({ membership: "removed", role: null }), base.id).kind).toBe("removed");
  });
});

describe("allows", () => {
  it("lets everyone but a Viewer, and nobody off the list", () => {
    for (const p of ACTIONS) {
      expect(allows({ kind: "owner" }, p)).toBe(true);
      expect(allows({ kind: "legacy" }, p)).toBe(true);
      for (const role of ["owner", "admin", "member"] as Role[]) {
        expect(allows({ kind: "role", role }, p)).toBe(true);
      }
      expect(allows({ kind: "role", role: "viewer" }, p)).toBe(false);
      expect(allows({ kind: "waiting" }, p)).toBe(false);
      expect(allows({ kind: "removed" }, p)).toBe(false);
    }
  });
});

describe("notAllowedText", () => {
  it("names the role and the action", () => {
    const viewer = { kind: "role", role: "viewer" } as const;
    expect(notAllowedText("Acme", viewer, "edit")).toBe(
      "Acme: a Viewer can't create or change dots. Ask an Admin or Owner.",
    );
    expect(notAllowedText("Acme", viewer, "run")).toBe(
      "Acme: a Viewer can't run dots. Ask an Admin or Owner.",
    );
    expect(notAllowedText("Acme", viewer, "answer")).toBe(
      "Acme: a Viewer can't answer approvals. Ask an Admin or Owner.",
    );
  });

  it("says why a PC off the list gets nothing", () => {
    expect(notAllowedText("Acme", { kind: "waiting" }, "edit")).toBe(
      "Acme: you aren't on its member list yet. Send your Dots ID to an Admin.",
    );
    expect(notAllowedText("Acme", { kind: "removed" }, "run")).toBe(
      "Acme: you were removed from it.",
    );
  });
});
