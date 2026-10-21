import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const [open, answered, dismissed, dotsNote] = sample.messages;
const note: Message = {
  ...dotsNote,
  id: "n1",
  run_id: "r1",
  read_at: null,
  text: "Half the tests pass.",
};
const asking = `Question from ${open.dot_name}`;

/// The messages the core holds; an answer, a dismiss or a read changes one, as in the core.
function holding(initial: Message[]) {
  let current = initial;
  const change = (id: unknown, fields: Partial<Message>) => {
    current = current.map((m) => (m.id === id ? { ...m, ...fields } : m));
    return current.find((m) => m.id === id);
  };
  backend.on("messages_list", () => current);
  backend.on("message_reply", (args) => {
    change(args.id, { answered_at: sample.run.queued_at, answer_run_id: "run-answer" });
    return { ...sample.run, id: "run-answer", trigger: "chat", status: "queued" };
  });
  backend.on("message_dismiss", (args) => change(args.id, { dismissed_at: sample.run.queued_at }));
  backend.on("message_mark_read", (args) => change(args.id, { read_at: sample.run.queued_at }));
}

describe("the Inbox's messages", () => {
  it("lists the open questions and unread notes first and counts them", async () => {
    holding([open, answered, dismissed, note]);
    backend.on("inbox", () => sample.inbox);
    renderApp("/inbox");
    const group = await screen.findByRole("region", { name: "Messages" });
    const approvals = screen.getByRole("region", { name: "Approvals" });
    expect(group.compareDocumentPosition(approvals) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(group).getAllByRole("listitem")).toHaveLength(2);
    const question = within(group).getByRole("listitem", { name: asking });
    expect(question).toHaveTextContent(open.text);
    expect(within(question).getByRole("button", { name: "yes" })).toBeVisible();
    expect(within(question).getByRole("button", { name: "no" })).toBeVisible();
    expect(
      within(group).getByRole("listitem", { name: `Note from ${note.dot_name}` }),
    ).toHaveTextContent("Half the tests pass.");
    // The four items of the sample Inbox and the two messages.
    const nav = screen.getByRole("navigation", { name: "Main" });
    await waitFor(() =>
      expect(within(nav).getByRole("link", { name: /Inbox/ })).toHaveTextContent("6"),
    );
    // A question shown here counts as read; it stays open.
    await waitFor(() => expect(backend.callsTo("message_mark_read")).toEqual([{ id: open.id }]));
    expect(within(group).getByRole("listitem", { name: asking })).toBeVisible();
  });

  it("answers a question with one of its choices", async () => {
    holding([open]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "yes" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([{ id: open.id, text: "yes" }]),
    );
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
  });

  it("answers a question with typed text", async () => {
    holding([{ ...open, choices: null }]);
    const { user } = renderApp("/inbox");
    const box = await screen.findByLabelText(`Answer to ${open.dot_name}: ${open.text}`);
    expect(screen.getByRole("button", { name: "Reply" })).toBeDisabled();
    await user.type(box, "  Ship it on Friday.  ");
    await user.click(screen.getByRole("button", { name: "Reply" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([
        { id: open.id, text: "Ship it on Friday." },
      ]),
    );
  });

  // Each reply box says which question it answers, so two questions of one dot differ.
  it("labels each reply box with its question", async () => {
    const other = { ...open, id: `${open.id}-2`, text: "Which branch?", choices: null };
    holding([{ ...open, choices: null }, other]);
    renderApp("/inbox");
    expect(await screen.findByLabelText(`Answer to ${open.dot_name}: ${open.text}`)).toBeVisible();
    expect(screen.getByLabelText(`Answer to ${open.dot_name}: Which branch?`)).toBeVisible();
  });

  // Review Focus 3.
  it("answers once on a double click", async () => {
    holding([open]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.dblClick(within(question).getByRole("button", { name: "no" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("message_reply")).toEqual([{ id: open.id, text: "no" }]);
  });

  it("dismisses a question and marks a note read", async () => {
    holding([open, note]);
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(backend.callsTo("message_dismiss")).toEqual([{ id: open.id }]));
    const shown = await screen.findByRole("listitem", { name: `Note from ${note.dot_name}` });
    await user.click(within(shown).getByRole("button", { name: "Mark read" }));
    expect(await screen.findByText(/Nothing waits for you/)).toBeVisible();
    expect(backend.callsTo("message_mark_read")).toContainEqual({ id: note.id });
  });

  it("shows a refused answer as the core words it", async () => {
    backend.on("messages_list", () => [open]);
    backend.on("message_reply", () => {
      throw { kind: "conflict", message: "this question has already been answered" };
    });
    const { user } = renderApp("/inbox");
    const question = await screen.findByRole("listitem", { name: asking });
    await user.click(within(question).getByRole("button", { name: "yes" }));
    expect(await screen.findByText("this question has already been answered")).toBeVisible();
    expect(within(question).getByRole("button", { name: "yes" })).toBeEnabled();
  });
});
