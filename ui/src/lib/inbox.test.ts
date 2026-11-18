import { describe, expect, it } from "vitest";

import contract from "@/api/__fixtures__/contract.json";
import * as T from "@/api/types";
import { hiddenKeys, inboxKeys, inboxWaiting, keyString, makeRequestKey, messageKey } from "@/lib/inbox";

const inbox = T.Inbox.parse(contract.Inbox);
const messages = T.Message.array().parse(contract.Message);

describe("Inbox item keys", () => {
  it("inboxKeys_match_the_core", () => {
    // The core's keys for the same Inbox and messages (`inbox::entries`), in the same order.
    expect(inboxKeys(inbox, messages)).toEqual(T.InboxKey.array().parse(contract.InboxKeys));
  });

  it("reads a job request as a make request, keyed by its id", () => {
    // A job for a made dot that holds more than its maker waits for the person as its own kind.
    const [made] = inbox.make_requests ?? [];
    const job = T.PendingMakeRequest.parse({ ...made, id: "01J0MRJ", kind: "job" });
    expect(job.kind).toBe("job");
    expect(makeRequestKey(job)).toEqual({ kind: "make_request", id: "01J0MRJ" });
  });

  it("puts snoozed, done and hidden messages in the hidden keys", () => {
    const hidden = hiddenKeys(inbox);
    for (const mark of [...inbox.snoozed, ...inbox.done]) {
      expect(hidden.has(keyString(mark))).toBe(true);
    }
    for (const mark of inbox.back) expect(hidden.has(keyString(mark))).toBe(false);
  });
});

describe("inboxWaiting", () => {
  // The Inbox with nothing hidden, and one message that waits.
  const shown: T.Inbox = { ...inbox, snoozed: [], done: [], back: [], hidden_messages: [] };
  const open = messages.filter((m) => m.answered_at === null && m.dismissed_at === null && m.kind === "question");

  it("counts every section, the held runs as one line, and the messages that wait", () => {
    expect(open).toHaveLength(1);
    const sections =
      shown.approvals.length +
      shown.not_delivered.length +
      shown.review_runs.length +
      shown.attention.length +
      shown.tools_changed.length +
      shown.watchers.length +
      1 +
      shown.instructions.length +
      shown.app_reviews.length +
      shown.team_scope.length +
      shown.workflows.length +
      (shown.make_requests?.length ?? 0);
    expect(shown.held.length).toBeGreaterThan(1);
    expect(shown.make_requests?.length).toBe(1);
    expect(shown.not_delivered.length).toBe(1);
    expect(shown.team_scope.length).toBe(1);
    expect(shown.workflows.length).toBe(1);
    expect(inboxWaiting(shown, messages)).toBe(sections + 1);
    expect(inboxWaiting(undefined, messages)).toBe(1);
    expect(inboxWaiting(undefined, undefined)).toBe(0);
  });

  it("leaves out a snoozed or done message", () => {
    const all = inboxWaiting(shown, messages);
    const mark = {
      ...messageKey(open[0]),
      title: "t",
      dot_id: null,
      snoozed_until: null,
      done_at: null,
      back_at: null,
    };
    const snoozed = { ...shown, snoozed: [{ ...mark, snoozed_until: "2026-11-05T09:00:00Z" }] };
    expect(inboxWaiting(snoozed, messages)).toBe(all - 1);
    const done = { ...shown, done: [{ ...mark, done_at: "2026-11-04T09:00:00Z" }] };
    expect(inboxWaiting(done, messages)).toBe(all - 1);
    // Done more than 7 days ago: no longer listed, and still hidden.
    const old = { ...shown, hidden_messages: [open[0].id] };
    expect(inboxWaiting(old, messages)).toBe(all - 1);
    // Back from snooze: counted again.
    const back = { ...shown, back: [{ ...mark, back_at: "2026-11-04T09:00:00Z" }] };
    expect(inboxWaiting(back, messages)).toBe(all);
  });
});
