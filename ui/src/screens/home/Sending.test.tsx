import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import type { Run } from "@/api/types";
import { localTime } from "@/lib/format";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { chat, dot, home, reply, scheduled } from "@/test/home";
import { renderApp } from "@/test/render";

const box = () => screen.getByLabelText("Message");
const target = () => screen.getByRole("group", { name: "Reply target" });
/// The message box, once the conversation has loaded and it can be used.
async function readyBox() {
  const found = await screen.findByLabelText("Message");
  await waitFor(() => expect(found).toBeEnabled());
  return found;
}

/// A browser takes the focus from a control that is disabled and leaves it on the page. jsdom
/// keeps it on the control and cannot blur it there, so the focus goes by way of another one.
function loseFocus() {
  const other = screen.getByRole("button", { name: "New topic" });
  act(() => {
    other.focus();
    other.blur();
  });
}

describe("sending on the home", () => {
  it("starts the first topic of a dot with its first message", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("New topic")).toBeVisible();
    await user.type(await readyBox(), "Why did the build fail?{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Why did the build fail?", parentRunId: null },
      ]),
    );
    const view = within(screen.getByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Why did the build fail?")).toBeVisible();
    expect(target()).toHaveTextContent("Replying to: current topic");
  });

  it("continues the current topic from its last run", async () => {
    const a = chat("a", "10:00");
    home([a, reply("a2", a, "10:05")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "And now?");
    expect(target()).toHaveTextContent("Replying to: current topic");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "And now?", parentRunId: "a2" },
      ]),
    );
    expect(box()).toHaveValue("");
    // The new run is queued: no second message until it ends.
    expect(await screen.findByText("And now?")).toBeVisible();
    await waitFor(() => expect(box()).toBeDisabled());
    expect(box()).toHaveAttribute("placeholder", `${dot.name} is working on this…`);
  });

  it("replies by default to the current topic, not a scheduled run that came after it", async () => {
    home([chat("a", "10:00"), scheduled("cron", "11:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Thanks{Enter}");
    expect(target()).toHaveTextContent("Replying to: current topic");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Thanks", parentRunId: "a" },
      ]),
    );
  });

  it("keeps the reply on the current topic when a scheduled run arrives while typing", async () => {
    const runs = home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Half a sen");
    const cron = { ...scheduled("cron", "11:00"), status: "succeeded" as const };
    runs.push(cron);
    act(() => backend.emit("runtime", { type: "run_updated", run: cron }));
    expect(
      await screen.findByRole("article", { name: `Scheduled · ${localTime(cron.queued_at)}` }),
    ).toBeVisible();
    expect(target()).toHaveTextContent("Replying to: current topic");
    expect(target()).toHaveAttribute("aria-live", "polite");
    await user.type(box(), "tence{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Half a sentence", parentRunId: "a" },
      ]),
    );
  });

  it("continues an older run topic opened from History", async () => {
    home([scheduled("old", "09:00"), chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}/old`);
    await user.type(await readyBox(), "About this run{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "About this run", parentRunId: "old" },
      ]),
    );
  });

  it("replies to the run topic whose Reply was pressed, until ✕", async () => {
    const one = scheduled("one", "11:00");
    const two = scheduled("two", "12:00");
    home([chat("a", "10:00"), one, two, reply("a2", { ...chat("a", "10:00") }, "13:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    expect(target()).toHaveTextContent("Replying to: current topic");
    const card = await screen.findByRole("article", {
      name: `Scheduled · ${localTime(one.queued_at)}`,
    });
    await user.click(within(card).getByRole("button", { name: "Reply" }));
    expect(target()).toHaveTextContent(`Replying to: Scheduled · ${localTime(one.queued_at)}`);
    await user.click(within(target()).getByRole("button", { name: "Clear the reply target" }));
    expect(target()).toHaveTextContent("Replying to: current topic");
    expect(within(target()).queryByRole("button")).toBeNull();

    await user.click(within(card).getByRole("button", { name: "Reply" }));
    await user.type(box(), "Why that one?{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Why that one?", parentRunId: "one" },
      ]),
    );
    // Sent: the target is the default again, the current chat topic.
    await waitFor(() => expect(target()).toHaveTextContent("Replying to: current topic"));
    expect(within(target()).queryByRole("button")).toBeNull();
  });

  it("starts a new topic, which becomes the current one", async () => {
    home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    await user.click(screen.getByRole("button", { name: "New topic" }));
    expect(target()).toHaveTextContent("New topic");
    // ✕ undoes it.
    await user.click(within(target()).getByRole("button", { name: "Clear the reply target" }));
    expect(target()).toHaveTextContent("Replying to: current topic");
    await user.click(screen.getByRole("button", { name: "New topic" }));
    expect(backend.callsTo("chat")).toEqual([]);

    await user.type(box(), "Something else{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Something else", parentRunId: null },
      ]),
    );
    const view = within(screen.getByRole("region", { name: "Conversation" }));
    expect(await view.findByText("Something else")).toBeVisible();
    // The old topic went to History.
    await waitFor(() => expect(view.queryByText("question a")).toBeNull());
    expect(target()).toHaveTextContent("Replying to: current topic");
  });

  it("lists older topics in History and continues one opened from there", async () => {
    const old = chat("old", "08:00");
    const cron = scheduled("cron", "09:00");
    home([old, cron, chat("current", "10:00")]);
    const { user, router } = renderApp(`/home/${dot.id}`);
    await readyBox();
    await user.click(screen.getByRole("button", { name: "History" }));
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      expect.stringMatching(new RegExp(`^Scheduled · ${localTime(cron.queued_at)}`)),
      expect.stringMatching(/^question old/),
    ]);
    await user.click(items[1]);
    await waitFor(() => expect(router.state.location.pathname).toBe(`/home/${dot.id}/old`));
    expect(await screen.findByText("Older topic ·", { exact: false })).toBeVisible();
    expect(target()).toHaveTextContent("Replying to: question old");

    await user.type(await readyBox(), "Back to this{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Back to this", parentRunId: "old" },
      ]),
    );
    expect(router.state.location.pathname).toBe(`/home/${dot.id}/old`);
    expect(await screen.findByText("Back to this")).toBeVisible();
  });

  it("says History has nothing older", async () => {
    home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await readyBox();
    await user.click(screen.getByRole("button", { name: "History" }));
    expect(await screen.findByRole("menuitem", { name: "No older topics" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("keeps the composer locked after a send until the topics have reloaded", async () => {
    const runs = home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    // From now on the thread reload is held until the test lets it answer.
    const held = later<void>();
    backend.on("run_thread", async (args) => {
      await held.answer;
      const root = runs.find((r) => r.id === args.runId)?.root_run_id;
      return runs.filter((r) => r.root_run_id === root);
    });

    await user.type(found, "first{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(1));
    await waitFor(() => expect(backend.callsTo("run_thread").length).toBeGreaterThan(1));
    expect(found).toBeDisabled();
    await user.type(found, "second{Enter}");
    expect(backend.callsTo("chat")).toHaveLength(1);

    await act(async () => held.give());
    expect(await screen.findByText("first")).toBeVisible();
    await waitFor(() =>
      expect(box()).toHaveAttribute("placeholder", `${dot.name} is working on this…`),
    );
    expect(box()).toBeDisabled();
  });

  it("holds the composer while the dot works and frees it when the run ends", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = home([working]);
    renderApp(`/home/${dot.id}`);
    const found = await screen.findByLabelText("Message");
    await waitFor(() => expect(found).toBeDisabled());
    expect(found).toHaveAttribute("placeholder", `${dot.name} is working on this…`);

    runs[0] = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    await waitFor(() => expect(box()).toBeEnabled());
  });

  it("says that the topic waits for an approval above", async () => {
    const waiting = scheduled("cron", "09:00", { status: "awaiting_approval", ended_at: null });
    home([waiting], [], [{ ...sample.approval, run_id: "cron" }]);
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await screen.findByLabelText("Message");
    // A new topic can start meanwhile; a reply to the waiting run cannot.
    await waitFor(() => expect(found).toBeEnabled());
    const card = await screen.findByRole("article", {
      name: `Scheduled · ${localTime(waiting.queued_at)}`,
    });
    await user.click(within(card).getByRole("button", { name: "Reply" }));
    await waitFor(() =>
      expect(found).toHaveAttribute("placeholder", "Waiting for your approval above"),
    );
    expect(found).toBeDisabled();
  });

  it("says that a topic whose last run ended without a session cannot be continued", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null, session_id: null });
    const runs = home([working]);
    const { user } = renderApp(`/home/${dot.id}`);
    await waitFor(() =>
      expect(box()).toHaveAttribute("placeholder", `${dot.name} is working on this…`),
    );
    // The run can still get its session.
    expect(screen.queryByText(/cannot be continued/)).toBeNull();

    runs[0] = {
      ...working,
      status: "failed",
      error: "the claude CLI was not found",
      ended_at: sample.run.ended_at,
    };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    const note = await screen.findByText(/This topic cannot be continued/);
    expect(box()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();

    await user.click(within(note.parentElement!).getByRole("button", { name: "New topic" }));
    expect(target()).toHaveTextContent("New topic");
    await user.type(await readyBox(), "Try again{Enter}");
    await waitFor(() =>
      expect(backend.callsTo("chat")).toEqual([
        { dotId: dot.id, message: "Try again", parentRunId: null },
      ]),
    );
  });

  it("gives the message box the focus back when the answer to a send has ended", async () => {
    const runs = home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    await user.type(found, "And now?{Enter}");
    await waitFor(() =>
      expect(found).toHaveAttribute("placeholder", `${dot.name} is working on this…`),
    );
    loseFocus();
    expect(found).not.toHaveFocus();

    runs[1] = { ...runs[1], status: "succeeded", ended_at: sample.run.ended_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[1] }));
    await waitFor(() => expect(found).toBeEnabled());
    expect(found).toHaveFocus();
  });

  it("keeps the text and gives the focus back after a refused send", async () => {
    home([chat("a", "10:00")]);
    const refusal = later<Run>();
    backend.on("chat", () => refusal.answer);
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    await user.type(found, "hello{Enter}");
    await waitFor(() => expect(found).toBeDisabled());
    loseFocus();

    await act(async () => refusal.fail({ kind: "conflict", message: "the dot is disabled" }));
    expect(await screen.findByText("the dot is disabled")).toBeVisible();
    await waitFor(() => expect(found).toBeEnabled());
    expect(found).toHaveFocus();
    expect(found).toHaveValue("hello");
  });

  it("leaves the focus where the user put it while the dot worked", async () => {
    const working = chat("a", "10:00", { status: "running", ended_at: null });
    const runs = home([working]);
    renderApp(`/home/${dot.id}`);
    const found = await screen.findByLabelText("Message");
    await waitFor(() => expect(found).toBeDisabled());
    const elsewhere = screen.getByRole("button", { name: "History" });
    act(() => elsewhere.focus());

    runs[0] = { ...working, status: "succeeded", ended_at: sample.run.ended_at };
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    await waitFor(() => expect(found).toBeEnabled());
    expect(elsewhere).toHaveFocus();
  });

  it("loads the topics again after a refusal and replies to the run the topic ends with", async () => {
    const runs = home([chat("a", "10:00")]);
    backend.on("chat", (args) => {
      if (backend.callsTo("chat").length === 1) {
        runs.push(reply("a2", runs[0], "10:05", { payload: { message: "asked elsewhere" } }));
        throw {
          kind: "conflict",
          message:
            "this chat has a newer message than the one you replied to; open the chat again, then send your message",
        };
      }
      const next = reply("a3", runs[1], "10:10", {
        status: "queued",
        payload: { message: args.message },
      });
      runs.push(next);
      return next;
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    await user.type(found, "hello{Enter}");
    expect(await screen.findByText(/this chat has a newer message/)).toBeVisible();
    expect(await screen.findByText("asked elsewhere")).toBeVisible();
    expect(found).toHaveValue("hello");

    await waitFor(() => expect(found).toBeEnabled());
    await user.type(found, "{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(2));
    expect(backend.callsTo("chat")[1]).toEqual({
      dotId: dot.id,
      message: "hello",
      parentRunId: "a2",
    });
  });

  it("writes a line break on Shift+Enter instead of sending", async () => {
    home([]);
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    await user.type(found, "one{Shift>}{Enter}{/Shift}two");
    expect(found).toHaveValue("one\ntwo");
    expect(backend.callsTo("chat")).toEqual([]);
  });

  it("sends to a disabled dot, whose conversation stays readable", async () => {
    const off = { ...dot, enabled: false };
    home([chat("a", "10:00")]);
    backend.on("dot_overview", () => [
      { dot: off, active_run: null, last_run: null, schedule_state: null, needs_you: 0, unread: 0 },
    ]);
    const { user } = renderApp(`/home/${dot.id}`);
    expect(await screen.findByText("question a")).toBeVisible();
    await user.type(await readyBox(), "Still there?{Enter}");
    await waitFor(() => expect(backend.callsTo("chat")).toHaveLength(1));
  });
});

describe("a chat while Claude's limit is reached", () => {
  it("says until when, and keeps the message", async () => {
    home([chat("a", "10:00")]);
    backend.on("chat", () => {
      throw { kind: "invalid", message: "Claude's limit is reached until 15:10." };
    });
    const { user } = renderApp(`/home/${dot.id}`);
    const found = await readyBox();
    await user.type(found, "hello{Enter}");
    expect(await screen.findByText("Claude's limit is reached until 15:10.")).toBeVisible();
    expect(found).toHaveValue("hello");
  });
});

describe("a failed reload of the dots while a conversation is open", () => {
  it("keeps the conversation and the text being typed, and says the load failed", async () => {
    const runs = home([chat("a", "10:00")]);
    const { user } = renderApp(`/home/${dot.id}`);
    await user.type(await readyBox(), "Draft that must survive");
    backend.on("dot_overview", () => {
      throw { kind: "internal", message: "database is locked" };
    });
    act(() => backend.emit("runtime", { type: "run_updated", run: runs[0] }));
    expect(await screen.findByRole("alert")).toHaveTextContent("database is locked");
    expect(box()).toHaveValue("Draft that must survive");
    expect(screen.getByRole("region", { name: "Conversation" })).toBeVisible();
  });
});
