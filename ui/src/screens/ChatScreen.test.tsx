import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Approval, Dot, Run, RunEventRecord } from "@/api/types";
import { backend, later } from "@/test/backend";
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

/// A browser takes the focus from a control that is disabled and leaves it on the page. jsdom
/// keeps it on the control and cannot blur it there, so the focus goes by way of another one.
function loseFocus() {
  const other = screen.getByLabelText("Dot");
  act(() => {
    other.focus();
    other.blur();
  });
}

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

  it("lists a dot created after Chat was first opened", async () => {
    const dots: Dot[] = [];
    backend.on("list_dots", () => dots);
    backend.on("get_dot", (args) => dots.find((d) => d.id === args.id));
    backend.on("create_dot", (args) => {
      const created: Dot = { ...dot, ...(args.spec as object), id: "dot-new" };
      dots.push(created);
      return created;
    });
    const { user, router } = renderApp("/chat");
    await user.click(await screen.findByRole("link", { name: "Create one" }));
    await user.type(await screen.findByLabelText("Name"), "nightly");
    await user.type(screen.getByLabelText("Instructions"), "Run the tests.");
    await user.type(screen.getByLabelText("Folder"), "C:/dev/app");
    await user.click(screen.getByRole("button", { name: "Create dot" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/dots/dot-new"));

    await user.click(screen.getByRole("link", { name: "Chat" }));
    expect(await screen.findByRole("option", { name: "nightly" })).toBeInTheDocument();
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
    const conversation = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await conversation.findByText("Why did the build fail?")).toBeVisible();
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

  it("says that a chat whose last run ended without a session cannot be continued", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null, session_id: null });
    const runs = chats([working]);
    const { user, router } = renderApp(`/chat/${dot.id}/a`);
    const conversation = within(await screen.findByRole("region", { name: "Conversation" }));
    await waitFor(() =>
      expect(screen.getByLabelText("Message")).toHaveAttribute("placeholder", "The dot is working…"),
    );
    // The run can still get its session.
    expect(conversation.queryByText(/cannot be continued/)).toBeNull();

    // It fails before the engine has one: the core would refuse every follow-up.
    const failed: Run = {
      ...working,
      status: "failed",
      summary: null,
      error: "the claude CLI was not found",
      ended_at: sample.run.ended_at,
    };
    runs[0] = failed;
    act(() => backend.emit("runtime", { type: "run_updated", run: failed }));
    expect(await conversation.findByText(/This chat cannot be continued/)).toBeVisible();
    expect(screen.getByLabelText("Message")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();

    await user.click(conversation.getByRole("link", { name: "Start a new chat" }));
    await waitFor(() => expect(router.state.location.pathname).toBe(`/chat/${dot.id}`));
    expect(await screen.findByLabelText("Message")).toBeEnabled();
  });

  it("gives the message box the focus back when the answer to a send has ended", async () => {
    const runs = chats([chat("a", "10:00")]);
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "And now?{Enter}");
    await waitFor(() => expect(box).toHaveAttribute("placeholder", "The dot is working…"));
    loseFocus();
    expect(box).not.toHaveFocus();

    const done: Run = { ...runs[1], status: "succeeded", ended_at: sample.run.ended_at };
    runs[1] = done;
    act(() => backend.emit("runtime", { type: "run_updated", run: done }));
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).toHaveFocus();
  });

  it("gives the message box the focus back, with its text, after a refused send", async () => {
    chats([chat("a", "10:00")]);
    const refusal = later<Run>();
    backend.on("chat", () => refusal.answer);
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "hello{Enter}");
    await waitFor(() => expect(box).toBeDisabled());
    loseFocus();
    expect(box).not.toHaveFocus();

    await act(async () => refusal.fail({ kind: "conflict", message: "the dot is disabled" }));
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).toHaveFocus();
    expect(box).toHaveValue("hello");
  });

  it("leaves the focus where the user put it while the dot worked", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = chats([working]);
    renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeDisabled());
    const picker = screen.getByLabelText("Dot");
    act(() => picker.focus());

    const done: Run = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    runs[0] = done;
    act(() => backend.emit("runtime", { type: "run_updated", run: done }));
    await waitFor(() => expect(box).toBeEnabled());
    expect(picker).toHaveFocus();
  });

  it("lists a chat that was started while the list of chats was loading", async () => {
    const runs = chats([]);
    const { user, router, listening } = renderApp("/chat");
    await screen.findByText("Choose a dot to chat with.");
    await listening();

    // The list of the dot's chats is read before the first message is sent, and answers after.
    const read = later<Run[]>();
    backend.on("list_runs", () => (backend.callsTo("list_runs").length === 1 ? read.answer : runs));
    await user.selectOptions(screen.getByLabelText("Dot"), dot.id);
    await user.type(await screen.findByLabelText("Message"), "Why did the build fail?{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe(`/chat/${dot.id}/run-1`));
    await act(async () => read.give([]));

    const list = within(screen.getByRole("navigation", { name: "Chats" }));
    expect(await list.findByRole("link", { name: /Why did the build fail/ })).toBeVisible();
    expect(backend.callsTo("list_runs")).toHaveLength(2);
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
        { id: sample.approval.id, approved: true, note: null, allowRule: null },
      ]),
    );
  });

  it("shows a failure once when the run also stored it as an event", async () => {
    chats(
      [chat("a", "10:00", { status: "failed", summary: null, error: "engine: boom" })],
      [
        said("a", 1, "Trying."),
        {
          run_id: "a",
          seq: 2,
          ts: sample.run.queued_at,
          kind: "failed",
          data: { kind: "failed", error: "engine: boom" },
        },
      ],
    );
    renderApp(`/chat/${dot.id}/a`);
    const conversation = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await conversation.findByText("Trying.")).toBeVisible();
    expect(conversation.getAllByRole("alert")).toHaveLength(1);
    expect(conversation.getByRole("alert")).toHaveTextContent("engine: boom");
  });

  it("shows a failed load of a conversation's events and keeps what did load", async () => {
    chats([chat("a", "10:00")]);
    backend.on("list_run_events", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/chat/${dot.id}/a`);
    const conversation = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await conversation.findByRole("alert")).toHaveTextContent("database is locked");
    expect(conversation.getByText("question a")).toBeVisible();
  });

  it("shows a failed load of a conversation's approvals", async () => {
    chats([chat("a", "10:00")]);
    backend.on("approvals_for_run", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    renderApp(`/chat/${dot.id}/a`);
    const conversation = within(await screen.findByRole("region", { name: "Conversation" }));
    expect(await conversation.findByRole("alert")).toHaveTextContent("database is locked");
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

  /// A core that holds one more run of thread "a" than the screen does, and so refuses the
  /// screen's next follow-up. `run_thread` answers with that run from the refusal on.
  function behind(newer: Partial<Run>, refusal: string) {
    const runs = chats([chat("a", "10:00")]);
    const thread = { root_run_id: "a", parent_run_id: "a" };
    backend.on("chat", () => {
      if (backend.callsTo("chat").length === 1) {
        runs.push(chat("a2", "10:05", { ...thread, ...newer }));
        throw { kind: "conflict", message: refusal };
      }
      const next = chat("a3", "10:10", { ...thread, parent_run_id: "a2", status: "queued" });
      runs.push(next);
      return next;
    });
    return runs;
  }

  it("loads the thread and the list of chats again when a follow-up is refused", async () => {
    behind(
      { payload: { message: "asked in another window" } },
      "this chat has a newer message than the one you replied to; open the chat again, then send your message",
    );
    const { user, listening } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await listening();
    const asked = { thread: backend.callsTo("run_thread").length, list: backend.callsTo("list_runs").length };

    await user.type(box, "hello{Enter}");
    expect(await screen.findByText(/this chat has a newer message/)).toBeVisible();
    // The run the screen did not know of is in the transcript, and the text is still there.
    const conversation = within(screen.getByRole("region", { name: "Conversation" }));
    expect(await conversation.findByText("asked in another window")).toBeVisible();
    expect(box).toHaveValue("hello");
    expect(backend.callsTo("run_thread")).toHaveLength(asked.thread + 1);
    expect(backend.callsTo("list_runs")).toHaveLength(asked.list + 1);

    // The next try replies to the run the thread really ends with.
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(2));
    expect(backend.callsTo("chat")[1]).toEqual({
      dotId: dot.id,
      message: "hello",
      parentRunId: "a2",
    });
  });

  it("says that the chat cannot be continued when a refusal brings a last run without a session", async () => {
    behind(
      { status: "failed", summary: null, error: "the claude CLI was not found", session_id: null },
      "this chat cannot be continued because its last run ended before the dot started working; start a new chat",
    );
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());
    await user.type(box, "hello{Enter}");

    const conversation = within(screen.getByRole("region", { name: "Conversation" }));
    expect(await conversation.findByText(/This chat cannot be continued/)).toBeVisible();
    expect(conversation.getByRole("link", { name: "Start a new chat" })).toBeVisible();
    expect(box).toBeDisabled();
    expect(box).toHaveValue("hello");
    // No second try goes out.
    await user.type(box, "{Enter}");
    expect(backend.callsTo("chat")).toHaveLength(1);
  });

  it("keeps the composer locked after a refused follow-up until the thread has reloaded", async () => {
    const runs = behind(
      { status: "running", ended_at: null },
      "the dot is still working on this chat; wait until it has finished, then send your message again",
    );
    const { user } = renderApp(`/chat/${dot.id}/a`);
    const box = await screen.findByLabelText("Message");
    await waitFor(() => expect(box).toBeEnabled());

    // From now on the thread reload is held until the test lets it answer.
    const read = later<Run[]>();
    backend.on("run_thread", () => read.answer);
    await user.type(box, "hello{Enter}");
    expect(await screen.findByText(/the dot is still working on this chat/)).toBeVisible();
    // Until the thread is here the screen still ends at the old run: no second try.
    expect(box).toBeDisabled();
    await user.type(box, "{Enter}");
    expect(backend.callsTo("chat")).toHaveLength(1);

    await act(async () => read.give(runs));
    await waitFor(() => expect(box).toHaveAttribute("placeholder", "The dot is working…"));
    expect(box).toBeDisabled();
    expect(box).toHaveValue("hello");
  });
});
