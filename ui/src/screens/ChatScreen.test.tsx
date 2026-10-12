import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, Run, RunEventRecord } from "@/api/types";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const dot = sample.dot;
const chat = (id: string, at: string, over: Partial<Run> = {}): Run => ({
  ...sample.run,
  id,
  root_run_id: id,
  parent_run_id: null,
  trigger: "chat",
  branch: null,
  payload: { message: `question ${id}` },
  queued_at: `2026-10-09T${at}:00.000Z`,
  ...over,
});
const said = (runId: string, seq: number, text: string): RunEventRecord => ({
  run_id: runId,
  seq,
  ts: sample.run.queued_at,
  kind: "assistant_text",
  data: { kind: "assistant_text", text },
});

/// A backend holding the dot's chat runs; `chat` appends a queued run like the core does.
function chats(runs: Run[], events: RunEventRecord[] = [], approvals: Approval[] = []) {
  const all = [...runs];
  backend.on("list_dots", () => [dot]);
  backend.on("list_runs", () => all);
  backend.on("run_thread", (args) => {
    const root = all.find((r) => r.id === args.runId)?.root_run_id;
    return all.filter((r) => r.root_run_id === root);
  });
  backend.on("list_run_events", (args) =>
    events.filter((e) => e.run_id === args.runId && e.seq > (args.afterSeq as number)),
  );
  backend.on("approvals_for_run", (args) => approvals.filter((a) => a.run_id === args.runId));
  backend.on("chat", (args) => {
    const parent = all.find((r) => r.id === args.parentRunId);
    const id = `run-${all.length + 1}`;
    const run = chat(id, "15:00", {
      status: "queued",
      ended_at: null,
      summary: null,
      payload: { message: args.message },
      parent_run_id: parent?.id ?? null,
      root_run_id: parent?.root_run_id ?? id,
    });
    all.push(run);
    return run;
  });
  return all;
}

describe("the Chat screen", () => {
  it("says so when there is no dot to chat with", async () => {
    renderApp("/chat");
    expect(await screen.findByText(/no dot to chat with yet/)).toBeVisible();
    expect(screen.getByRole("link", { name: "Create one" })).toHaveAttribute("href", "/dots/new");
  });

  it("asks for a dot first", async () => {
    chats([]);
    const { user, router } = renderApp("/chat");
    expect(await screen.findByText("Choose a dot to chat with.")).toBeVisible();
    await user.selectOptions(screen.getByLabelText("Dot"), dot.id);
    await waitFor(() => expect(router.state.location.pathname).toBe(`/chat/${dot.id}`));
    expect(await screen.findByLabelText("Message")).toBeEnabled();
  });

  it("lists the dot's conversations and opens one as a transcript", async () => {
    chats(
      [chat("a", "10:00"), chat("b", "11:00"), chat("cron", "12:00", { trigger: "schedule" })],
      [said("a", 1, "Answer to a.")],
    );
    const { user } = renderApp(`/chat/${dot.id}`);
    const list = await screen.findByRole("navigation", { name: "Chats" });
    await within(list).findByRole("link", { name: /question b/ });
    expect(
      within(list)
        .getAllByRole("link")
        .map((a) => a.getAttribute("href")),
    ).toEqual([`/chat/${dot.id}`, `/chat/${dot.id}/b`, `/chat/${dot.id}/a`]);

    await user.click(within(list).getByRole("link", { name: /question a/ }));
    const conversation = screen.getByRole("region", { name: "Conversation" });
    expect(await within(conversation).findByText("question a")).toBeVisible();
    expect(await within(conversation).findByText("Answer to a.")).toBeVisible();
    expect(within(list).getByRole("link", { name: /question a/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("starts a new conversation with the first message", async () => {
    chats([]);
    const { user, router } = renderApp(`/chat/${dot.id}`);
    await user.type(await screen.findByLabelText("Message"), "Why did the build fail?{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe(`/chat/${dot.id}/run-1`));
    expect(backend.callsTo("chat")).toEqual([
      { dotId: dot.id, message: "Why did the build fail?", parentRunId: null },
    ]);
    expect(await screen.findByText("Why did the build fail?")).toBeVisible();
  });

  it("continues a conversation from its last run", async () => {
    chats([
      chat("a", "10:00"),
      chat("a2", "10:05", { root_run_id: "a", parent_run_id: "a", payload: { message: "more" } }),
    ]);
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "And now?");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "And now?", parentRunId: "a2" },
      ]),
    );
    expect(box).toHaveValue("");
    // The new run is queued: no second message until it ends.
    expect(await screen.findByText("And now?")).toBeVisible();
    await waitFor(() => expect(screen.getByLabelText("Message")).toBeDisabled());
  });

  it("keeps the composer locked after a send until the thread has reloaded", async () => {
    const runs = chats([chat("a", "10:00")]);
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());

    // From now on the thread reload is held until the test lets it answer.
    let release: () => void = () => {};
    const held = new Promise<void>((resolve) => {
      release = resolve;
    });
    backend.on("run_thread", async (args) => {
      await held;
      const root = runs.find((r) => r.id === args.runId)?.root_run_id;
      return runs.filter((r) => r.root_run_id === root);
    });

    await user.type(box, "first{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(1));
    await waitFor(() => expect(backend.callsTo("run_thread").length).toBeGreaterThan(1));
    expect(box).toBeDisabled();
    await user.type(box, "second{Enter}");
    expect(backend.callsTo("chat")).toHaveLength(1);

    act(() => release());
    expect(await screen.findByText("first")).toBeVisible();
    // The new run is queued, so the box stays disabled for the run's sake.
    await waitFor(() => expect(screen.getByLabelText("Message")).toHaveAttribute("placeholder", "The dot is working…"));
    expect(screen.getByLabelText("Message")).toBeDisabled();
  });

  it("holds the composer while the dot works and frees it when the run ends", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = chats([working]);
    renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeDisabled());
    expect(box).toHaveAttribute("placeholder", "The dot is working…");

    const done: Run = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    runs[0] = done;
    act(() => backend.emit("runtime", { type: "run_updated", run: done }));
    await waitFor(() => expect(screen.getByLabelText("Message")).toBeEnabled());
  });

  it("writes a line break on Shift+Enter instead of sending", async () => {
    chats([]);
    const { user } = renderApp(`/chat/${dot.id}`);
    const box = await screen.findByLabelText("Message");
    await user.type(box, "one{Shift>}{Enter}{/Shift}two");
    expect(box).toHaveValue("one\ntwo");
    expect(backend.callsTo("chat")).toEqual([]);
  });

  it("asks for a pending approval inside the conversation", async () => {
    const waiting = chat("a", "10:00", { status: "awaiting_approval", ended_at: null });
    chats([waiting], [], [{ ...sample.approval, run_id: "a" }]);
    backend.on("decide_approval", () => ({ ...sample.approval, run_id: "a", status: "approved" }));
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const card = await screen.findByRole("article", { name: "Approval for Bash" });
    await user.click(within(card).getByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(backend.callsTo("decide_approval")).toEqual([
        { id: sample.approval.id, approved: true, note: null },
      ]),
    );
  });

  it("shows why a message could not be sent and keeps the conversation", async () => {
    chats([chat("a", "10:00")]);
    backend.on("chat", () => {
      throw { kind: "conflict", message: "the dot is disabled" };
    });
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "hello{Enter}");
    expect(await screen.findByText("the dot is disabled")).toBeVisible();
    expect(screen.getByLabelText("Message")).toHaveValue("hello");
    const conversation = screen.getByRole("region", { name: "Conversation" });
    expect(within(conversation).getByText("question a")).toBeVisible();
  });
});
