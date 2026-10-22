import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply } from "@/test/home";
import { renderApp } from "@/test/render";

const [open] = sample.messages; // a question with the choices "yes" and "no"
const asked: Message = { ...open, run_id: "a", created_at: "2026-10-09T10:02:00.000Z" };
const a = chat("a", "10:00");

describe("check-ins in the conversation", () => {
  it("shows the dot's notes and questions after the run that posted them", async () => {
    const note: Message = {
      ...sample.messages[3],
      id: "n1",
      run_id: "a",
      text: "Half the tests pass.",
      created_at: "2026-10-09T10:01:00.000Z",
    };
    home([a, reply("b", a, "11:00")]);
    backend.on("messages_list", () => [asked, note]);
    renderApp(`/home/${dot.id}`);
    const view = await screen.findByRole("region", { name: "Conversation" });
    await within(view).findByRole("group", { name: "Note from the dot" });
    const [first, second] = within(view).getAllByRole("region", { name: /^Chat message,/ });
    expect(within(first).getByRole("group", { name: "Note from the dot" })).toHaveTextContent(
      "Half the tests pass.",
    );
    const question = within(first).getByRole("group", { name: "Question from the dot" });
    expect(question).toHaveTextContent(asked.text);
    expect(question).toHaveTextContent("Question · waiting for your answer");
    expect(within(second).queryByRole("group", { name: /from the dot/ })).toBeNull();
  });

  it("replies to the open question from the bar above the composer", async () => {
    home([a]);
    backend.on("messages_list", () => [asked]);
    backend.on("message_reply", (args) =>
      reply("c", a, "12:00", {
        status: "queued",
        payload: { message: `Answer to your question "${asked.text}": ${String(args.text)}` },
      }),
    );
    const { user } = renderApp(`/home/${dot.id}`);
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
    // Opening the conversation marked the question read.
    expect(backend.callsTo("message_mark_read")).toEqual([{ id: asked.id }]);
  });

  it("answers the open question with one of its choices", async () => {
    home([a]);
    backend.on("messages_list", () => [asked]);
    backend.on("message_reply", () => reply("c", a, "12:00", { status: "queued" }));
    const { user } = renderApp(`/home/${dot.id}`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await user.click(within(bar).getByRole("button", { name: "yes" }));
    await waitFor(() =>
      expect(backend.callsTo("message_reply")).toEqual([{ id: asked.id, text: "yes" }]),
    );
  });

  // The conversation shows what the person typed, not the words Dots wraps it in for the dot.
  it("shows an answer as the person wrote it", async () => {
    const answered: Message = { ...asked, answered_at: asked.created_at, answer_run_id: "b" };
    home([
      a,
      reply("b", a, "11:00", {
        payload: { message: `Answer to your question "${asked.text}": Friday`, answer: "Friday" },
      }),
    ]);
    backend.on("messages_list", () => [answered]);
    renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("Friday")).toBeVisible();
    expect(screen.queryByText(/Answer to your question/)).toBeNull();
  });

  it("shows no bar once the question is answered, and a message goes to the topic", async () => {
    const answered: Message = {
      ...asked,
      read_at: asked.created_at,
      answered_at: asked.created_at,
      answer_run_id: "b",
    };
    home([a]);
    backend.on("messages_list", () => [answered]);
    const { user } = renderApp(`/home/${dot.id}`);
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

  it("opens the conversation when an answer in an older topic started a new one", async () => {
    const old = chat("old", "08:00", { session_id: null, status: "failed" });
    home([old, chat("current", "10:00")]);
    const question: Message = { ...asked, run_id: "old" };
    backend.on("messages_list", () => [question]);
    backend.on("message_reply", () => chat("fresh", "12:00", { status: "queued" }));
    const { user, router } = renderApp(`/home/${dot.id}/old`);
    const bar = await screen.findByRole("region", { name: "Open question" });
    await user.click(within(bar).getByRole("button", { name: "no" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
  });
});
