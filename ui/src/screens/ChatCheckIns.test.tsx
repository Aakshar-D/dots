import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message, Run } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
/// A chat run of the thread that starts at run "a".
const run = (id: string, at: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: "a",
  parent_run_id: null,
  trigger: "chat",
  branch: null,
  payload: { message: `question ${id}` },
  queued_at: `2026-10-09T${at}:00.000Z`,
  ...over,
});
const [open] = sample.messages; // a question with the choices "yes" and "no"
const asked: Message = { ...open, run_id: "a", created_at: "2026-10-09T10:02:00.000Z" };

/// A dot with one chat thread of `runs`, and its `messages`.
function thread(runs: Run[], messages: Message[]) {
  backend.on("list_dots", () => [dot]);
  backend.on("list_runs", () => runs);
  backend.on("run_thread", () => runs);
  backend.on("list_run_events", () => []);
  backend.on("approvals_for_run", () => []);
  backend.on("messages_list", () => messages);
}

describe("check-ins in a chat", () => {
  it("shows the dot's notes and questions after the run that posted them", async () => {
    const note: Message = {
      ...sample.messages[3],
      id: "n1",
      run_id: "a",
      text: "Half the tests pass.",
      created_at: "2026-10-09T10:01:00.000Z",
    };
    thread([run("a", "10:00"), run("b", "11:00", { parent_run_id: "a" })], [asked, note]);
    renderApp(`/chat/${dot.id}/a`);
    const conversation = await screen.findByRole("region", { name: "Conversation" });
    await within(conversation).findByRole("group", { name: "Note from the dot" });
    const [first, second] = within(conversation).getAllByRole("region", {
      name: /^Chat message,/,
    });
    expect(within(first).getByRole("group", { name: "Note from the dot" })).toHaveTextContent(
      "Half the tests pass.",
    );
    const question = within(first).getByRole("group", { name: "Question from the dot" });
    expect(question).toHaveTextContent(asked.text);
    expect(question).toHaveTextContent("Question · waiting for your answer");
    expect(within(second).queryByRole("group", { name: /from the dot/ })).toBeNull();
  });

  it("replies to the open question from the bar above the composer", async () => {
    thread([run("a", "10:00")], [asked]);
    backend.on("message_reply", (args) =>
      run("c", "12:00", {
        parent_run_id: "a",
        status: "queued",
        payload: { message: `Answer to your question "${asked.text}": ${String(args.text)}` },
      }),
    );
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    expect(bar).toHaveTextContent(`${dot.name} asked: ${asked.text}`);
    await user.click(within(bar).getByRole("button", { name: "Reply to question" }));
    await user.type(screen.getByLabelText("Message"), "Ship it on Friday.{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([
        { id: asked.id, text: "Ship it on Friday." },
      ]),
    );
    expect(backend.callsTo("chat")).toEqual([]);
    // Opening the chat marked the question read.
    expect(backend.callsTo("message_mark_read")).toEqual([{ id: asked.id }]);
  });

  // The chat shows what the person typed, not the words Dots wraps it in for the dot.
  it("shows an answer as the person wrote it", async () => {
    const answered: Message = { ...asked, answered_at: asked.created_at, answer_run_id: "b" };
    thread(
      [
        run("a", "10:00"),
        run("b", "11:00", {
          parent_run_id: "a",
          payload: { message: `Answer to your question "${asked.text}": Friday`, answer: "Friday" },
        }),
      ],
      [answered],
    );
    renderApp(`/chat/${dot.id}/b`);
    expect(await screen.findByText("Friday")).toBeVisible();
    expect(screen.queryByText(/Answer to your question/)).toBeNull();
  });

  it("answers the open question with one of its choices", async () => {
    thread([run("a", "10:00")], [asked]);
    backend.on("message_reply", () => run("c", "12:00", { parent_run_id: "a", status: "queued" }));
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await user.click(within(bar).getByRole("button", { name: "yes" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([{ id: asked.id, text: "yes" }]),
    );
  });

  it("shows no bar once the question is answered, and a message goes to the chat", async () => {
    const answered: Message = {
      ...asked,
      read_at: asked.created_at,
      answered_at: asked.created_at,
      answer_run_id: "b",
    };
    thread([run("a", "10:00")], [answered]);
    backend.on("chat", () => run("b", "12:00", { parent_run_id: "a", status: "queued" }));
    const { user } = renderApp(`/chat/${dot.id}/a`);
    expect(await screen.findByText("Question · answered")).toBeVisible();
    expect(screen.queryByRole("region", { name: "Open question" })).toBeNull();
    await user.type(screen.getByLabelText("Message"), "and now?{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "and now?", parentRunId: "a" },
      ]),
    );
    expect(backend.callsTo("message_reply")).toEqual([]);
  });
});
