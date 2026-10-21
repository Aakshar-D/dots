import { describe, expect, it } from "vitest";

import { createQueryClient, keys } from "@/lib/query";
import { applyRuntimeEvent } from "@/lib/runtime-events";
import { sample } from "@/test/fixtures";

type Client = ReturnType<typeof createQueryClient>;
const stale = (qc: Client, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

describe("check-in events and the query cache", () => {
  it("marks every list of messages stale when a message is posted or changes", () => {
    for (const type of ["message_posted", "message_updated"] as const) {
      const qc = createQueryClient();
      qc.setQueryData(keys.messages(null), []);
      qc.setQueryData(keys.messages(sample.dot.id), []);
      qc.setQueryData(keys.followUps(null), []);
      applyRuntimeEvent(qc, { type, message: sample.messages[0] });
      expect(stale(qc, keys.messages(null))).toBe(true);
      expect(stale(qc, keys.messages(sample.dot.id))).toBe(true);
      expect(stale(qc, keys.followUps(null))).toBe(false);
    }
  });

  it("marks every list of follow-ups stale when a dot's follow-ups change", () => {
    const qc = createQueryClient();
    qc.setQueryData(keys.followUps(null), []);
    qc.setQueryData(keys.followUps(sample.dot.id), []);
    qc.setQueryData(keys.messages(null), []);
    applyRuntimeEvent(qc, { type: "follow_ups_updated", dot_id: sample.dot.id });
    expect(stale(qc, keys.followUps(null))).toBe(true);
    expect(stale(qc, keys.followUps(sample.dot.id))).toBe(true);
    expect(stale(qc, keys.messages(null))).toBe(false);
  });
});
