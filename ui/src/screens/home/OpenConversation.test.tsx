import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Message, PendingApproval } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply } from "@/test/home";
import { renderApp } from "@/test/render";

const old = chat("old", "08:00");
const current = chat("current", "10:00");
const question: Message = { ...sample.messages[0], dot_id: dot.id, run_id: "current" };

describe("the Inbox's Open conversation", () => {
  it("opens the conversation a question of the current topic is in", async () => {
    home([old, current]);
    backend.on("messages_list", () => [question]);
    const { user, router } = renderApp("/inbox");
    const card = await screen.findByRole("listitem", { name: `Question from ${question.dot_name}` });
    expect(within(card).getByRole("link", { name: "Open the run" })).toHaveAttribute(
      "href",
      "/runs/current",
    );
    await user.click(within(card).getByRole("link", { name: "Open conversation" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
    expect(await screen.findByRole("region", { name: "Open question" })).toHaveTextContent(
      question.text,
    );
  });

  it("opens an older topic alone for an approval its run waits for", async () => {
    const waiting = reply("old2", old, "08:30", { status: "awaiting_approval", ended_at: null });
    home([old, waiting, current]);
    const approval: PendingApproval = {
      ...sample.inbox.approvals[0]!,
      run_id: "old2",
      dot_id: dot.id,
      dot_name: dot.name,
    };
    backend.on("inbox", () => ({
      approvals: [approval],
      review_runs: [],
      attention: [],
      tools_changed: [],
    }));
    const { user, router } = renderApp("/inbox");
    const group = await screen.findByRole("group", { name: `Approvals asked by ${dot.name}` });
    await user.click(within(group).getByRole("link", { name: "Open conversation" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}/old`));
    expect(await screen.findByText("Back to the current conversation")).toBeVisible();
  });

  it("opens the dot's conversation for a note of Dots' own", async () => {
    home([]);
    const note: Message = { ...sample.messages[3], dot_id: dot.id, run_id: null, read_at: null };
    backend.on("messages_list", () => [note]);
    const { user, router } = renderApp("/inbox");
    const card = await screen.findByRole("listitem", { name: `Note from ${note.dot_name}` });
    await user.click(within(card).getByRole("link", { name: "Open conversation" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}`));
  });

  it("drops the conversation's question bar when the question is answered in the Inbox", async () => {
    home([current]);
    let messages = [question];
    backend.on("messages_list", () => messages);
    const { listening } = renderApp(`/home/${dot.id}`);
    await screen.findByRole("region", { name: "Open question" });
    await listening();
    const answered = { ...question, answered_at: sample.run.ended_at, read_at: sample.run.ended_at };
    messages = [answered];
    act(() => backend.emit("runtime", { type: "message_updated", message: answered }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Open question" })).toBeNull());
  });
});
