import { describe, expect, it } from "vitest";

import { commands } from "@/api/commands";
import { createQueryClient, keys } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

type Client = ReturnType<typeof createQueryClient>;
const stale = (qc: Client, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

describe("the usage and limits commands", () => {
  it("read the limits and the usage, and release a held run, as the core names them", async () => {
    const [holding] = sample.limits;
    backend.on("limits_get", () => holding);
    backend.on("usage_summary", () => sample.usage);
    backend.on("run_release_hold", () => sample.run);
    expect(await commands.limitsGet()).toEqual(holding);
    expect(await commands.usageSummary("week")).toEqual(sample.usage);
    expect(await commands.runReleaseHold("run-1")).toEqual(sample.run);
    expect(backend.calls).toEqual([
      ["limits_get", {}],
      ["usage_summary", { period: "week" }],
      ["run_release_hold", { runId: "run-1" }],
    ]);
  });

  it("read a hold with its held runs, and a quiet state", () => {
    const [holding, quiet] = sample.limits;
    expect(holding?.holding).toBe(true);
    expect(holding?.held_run_ids.length).toBeGreaterThan(0);
    expect(quiet?.holding).toBe(false);
    expect(sample.inbox.held.length).toBeGreaterThan(0);
  });
});

describe("limits_updated", () => {
  it("writes the limits and marks the Inbox and the Usage page stale", () => {
    const qc = createQueryClient();
    const [holding, quiet] = sample.limits;
    qc.setQueryData(keys.limits, quiet);
    qc.setQueryData(keys.inbox, sample.inbox);
    qc.setQueryData(keys.usage("week"), sample.usage);
    applyRuntimeEvent(qc, { type: "limits_updated", limits: holding!, change: null });
    expect(qc.getQueryData(keys.limits)).toEqual(holding);
    expect(stale(qc, keys.inbox)).toBe(true);
    expect(stale(qc, keys.usage("week"))).toBe(true);
  });
});
