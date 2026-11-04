import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Inbox, InboxKey, InboxMark, Message } from "@/api/types";
import {
  approvalKey,
  inboxKeys,
  keyString,
  messageKey,
  reviewKey,
  watcherKey,
} from "@/lib/inbox";
import { backText } from "@/lib/later";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const question: Message = {
  ...sample.messages[0],
  id: "m1",
  dot_name: "courier",
  text: "Ship it?",
  read_at: "2026-11-04T09:00:00Z",
  choices: null,
};
const empty: Inbox = {
  approvals: [],
  review_runs: [],
  attention: [],
  tools_changed: [],
  watchers: [],
  held: [],
  instructions: [],
  app_reviews: [],
  team_scope: [],
  not_delivered: [],
  back: [],
  snoozed: [],
  done: [],
  hidden_messages: [],
};
const [watcher] = sample.inbox.watchers;
const [run] = sample.inbox.review_runs;
const [undelivered] = sample.inbox.not_delivered;

/// The core as far as the Inbox goes: the whole Inbox and the messages, and the marks the
/// person put on them. `inbox` answers each section without the hidden items, and the Back,
/// Snoozed and Done lists; the four commands change the marks.
function core(full: Inbox, messages: Message[], marks: InboxMark[] = []) {
  const held = new Map(marks.map((m) => [keyString(m), m]));
  const title = (key: InboxKey) => {
    const m = messages.find((x) => x.id === key.id);
    if (key.kind === "message" && m) {
      return `${m.dot_name} ${m.kind === "question" ? "asks" : "says"}: ${m.text}`;
    }
    return keyString(key);
  };
  const mark = (key: InboxKey, state: Partial<InboxMark>): InboxMark => ({
    ...key,
    title: title(key),
    dot_id: "d1",
    snoozed_until: null,
    done_at: null,
    back_at: null,
    ...state,
  });
  backend.on("messages_list", () => messages);
  backend.on("message_mark_read", (args) => messages.find((m) => m.id === args.id));
  backend.on("inbox", () => {
    const hidden = (k: InboxKey) => {
      const m = held.get(keyString(k));
      return m !== undefined && (m.snoozed_until !== null || m.done_at !== null);
    };
    const all = [...held.values()];
    return {
      ...full,
      approvals: full.approvals.filter((a) => !hidden(approvalKey(a))),
      review_runs: full.review_runs.filter((r) => !hidden(reviewKey(r))),
      watchers: full.watchers.filter((w) => !hidden(watcherKey(w))),
      back: all.filter((m) => m.back_at !== null),
      snoozed: all.filter((m) => m.snoozed_until !== null),
      done: all.filter((m) => m.done_at !== null),
      hidden_messages: messages.filter((m) => hidden(messageKey(m))).map((m) => m.id),
    };
  });
  backend.on("inbox_snooze", (args) => {
    for (const key of args.items as InboxKey[]) {
      held.set(keyString(key), mark(key, { snoozed_until: args.until as string }));
    }
    return null;
  });
  backend.on("inbox_done", (args) => {
    const key = args.item as InboxKey;
    held.set(keyString(key), mark(key, { done_at: new Date().toISOString() }));
    return null;
  });
  for (const cmd of ["inbox_unsnooze", "inbox_undo_done"]) {
    backend.on(cmd, (args) => {
      held.delete(keyString(args.item as InboxKey));
      return null;
    });
  }
  // Every key is one the core knows.
  expect(inboxKeys(full, messages).length).toBeGreaterThan(0);
}

/// Picks `preset` from the Snooze menu of `within`.
async function snooze(user: ReturnType<typeof renderApp>["user"], where: HTMLElement, preset: string) {
  await user.click(within(where).getByRole("button", { name: "Snooze" }));
  await user.click(await screen.findByRole("menuitem", { name: preset }));
}

describe("Snooze and Done in the Inbox", () => {
  it("snoozing_a_question_moves_it_to_snoozed", async () => {
    core(empty, [question]);
    const { user } = renderApp("/inbox");
    const card = await screen.findByRole("listitem", { name: "Question from courier" });
    await snooze(user, card, "In 1 hour");

    const toggle = await screen.findByRole("button", { name: "Snoozed (1)" });
    expect(screen.queryByRole("region", { name: "Messages" })).toBeNull();
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    const [{ until }] = backend.callsTo("inbox_snooze") as [{ until: string }];
    const list = screen.getByRole("list", { name: "Snoozed" });
    expect(within(list).getByRole("listitem")).toHaveTextContent(
      `courier asks: Ship it? · ${backText(until, Date.now())}`,
    );
    await user.click(within(list).getByRole("button", { name: "Unsnooze" }));
    await waitFor(() =>
      expect(backend.callsTo("inbox_unsnooze")).toEqual([{ item: { kind: "message", id: "m1" } }]),
    );
    expect(await screen.findByRole("region", { name: "Messages" })).toBeVisible();
    expect(screen.queryByRole("button", { name: /^Snoozed/ })).toBeNull();
  });

  it("done_moves_it_to_done_with_undo", async () => {
    core({ ...empty, watchers: [watcher] }, []);
    const { user } = renderApp("/inbox");
    const row = await screen.findByRole("listitem", { name: `${watcher.name} paused` });
    await user.click(within(row).getByRole("button", { name: "Done" }));
    const key = { kind: "watcher", id: `${watcher.watcher_id}@${watcher.at}` };
    await waitFor(() => expect(backend.callsTo("inbox_done")).toEqual([{ item: key }]));

    await user.click(await screen.findByRole("button", { name: "Done (1)" }));
    const list = screen.getByRole("list", { name: "Done" });
    expect(within(list).getByRole("listitem")).toHaveTextContent(/ · Done just now/);
    await user.click(within(list).getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(backend.callsTo("inbox_undo_done")).toEqual([{ item: key }]));
    expect(await screen.findByRole("listitem", { name: `${watcher.name} paused` })).toBeVisible();
  });

  it("approvals_have_no_done", async () => {
    const second = { ...sample.inbox.approvals[0], id: "a2" };
    core({ ...empty, approvals: [sample.inbox.approvals[0], second] }, []);
    const { user } = renderApp("/inbox");
    const group = await screen.findByRole("group", {
      name: `Approvals asked by ${second.dot_name}`,
    });
    expect(within(group).queryByRole("button", { name: "Done" })).toBeNull();
    await snooze(user, group, "In 1 hour");
    await waitFor(() =>
      expect(backend.callsTo("inbox_snooze").map((a) => a.items)).toEqual([
        [
          { kind: "approval", id: sample.inbox.approvals[0].id },
          { kind: "approval", id: "a2" },
        ],
      ]),
    );
    expect(await screen.findByRole("button", { name: "Snoozed (2)" })).toBeVisible();
    expect(screen.queryByRole("region", { name: "Approvals" })).toBeNull();
  });

  it("back_items_come_first_with_their_tag", async () => {
    const back: InboxMark = {
      ...reviewKey(run),
      title: "nightly-tests: run to review",
      dot_id: run.dot_id,
      snoozed_until: null,
      done_at: null,
      back_at: "2026-11-04T10:00:00Z",
    };
    core({ ...empty, review_runs: [run], watchers: [watcher] }, [question], [back]);
    renderApp("/inbox");
    const first = await screen.findByRole("region", { name: "Back from snooze" });
    expect(screen.getAllByRole("region")[0]).toBe(first);
    const row = within(first).getByRole("listitem", { name: `${run.dot_name} run to review` });
    expect(row).toBeVisible();
    expect(within(first).getByText("Back from snooze", { selector: "[data-slot=badge]" })).toBeVisible();
    // Not listed twice.
    expect(screen.queryByRole("region", { name: "Runs to review" })).toBeNull();
    expect(screen.getByRole("region", { name: "Messages" })).toBeVisible();
    expect(screen.getByRole("region", { name: "Paused watchers" })).toBeVisible();
  });

  it("not_delivered_section_lists_the_message", async () => {
    core({ ...empty, not_delivered: [undelivered] }, []);
    renderApp("/inbox");
    const section = await screen.findByRole("region", { name: "Not delivered" });
    const row = within(section).getByRole("listitem", {
      name: `${undelivered.dot_name} didn't get your message`,
    });
    expect(row).toHaveTextContent("Not delivered: claude CLI not found");
    expect(within(row).getByRole("button", { name: "Retry" })).toBeVisible();
    expect(within(row).getByRole("button", { name: "Discard" })).toBeVisible();
    expect(within(row).getByRole("link", { name: "Open" })).toHaveAttribute(
      "href",
      `/chat/${undelivered.dot_id}/${undelivered.id}`,
    );
    expect(within(row).getByRole("button", { name: "Done" })).toBeVisible();
  });

  it("everything_hidden_shows_the_empty_state_and_the_lists", async () => {
    const at = "2026-11-05T09:00:00Z";
    core({ ...empty, watchers: [watcher] }, [question], [
      {
        ...messageKey(question),
        title: "courier asks: Ship it?",
        dot_id: question.dot_id,
        snoozed_until: at,
        done_at: null,
        back_at: null,
      },
      {
        ...watcherKey(watcher),
        title: "nightly-tests's watcher Billing mail paused",
        dot_id: watcher.dot_id,
        snoozed_until: null,
        done_at: "2026-11-04T09:00:00Z",
        back_at: null,
      },
    ]);
    renderApp("/inbox");
    expect(await screen.findByText(/Nothing waits for you/)).toHaveTextContent(
      "messages that did not reach a dot",
    );
    expect(screen.getByRole("button", { name: "Snoozed (1)" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Done (1)" })).toBeVisible();
  });

  it("the_rail_count_leaves_out_a_snoozed_message", async () => {
    core(empty, [question]);
    const { user } = renderApp("/inbox");
    const rail = await screen.findByRole("navigation", { name: "Main" });
    expect(await within(rail).findByRole("link", { name: "Inbox, 1 waiting" })).toBeVisible();
    const card = await screen.findByRole("listitem", { name: "Question from courier" });
    await snooze(user, card, "Tomorrow morning (9 AM)");
    expect(await within(rail).findByRole("link", { name: "Inbox" })).toBeVisible();
  });
});
