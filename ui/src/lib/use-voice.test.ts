import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { MemoryRouter, useLocation, useNavigate } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { VoiceEvent } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { useVoice, useVoiceStatus, voiceStore } from "@/lib/use-voice";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";

/// Mounts `useVoice` at `path`, and hands back where the app is and a way to go elsewhere.
function mount(path: string, qc: QueryClient = createQueryClient()) {
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: qc },
      createElement(MemoryRouter, { initialEntries: [path] }, children),
    );
  const hook = renderHook(
    () => {
      useVoice();
      return { location: useLocation(), navigate: useNavigate() };
    },
    { wrapper },
  );
  return { ...hook, qc };
}

/// Emits `event` until the hook has heard it: its listener attaches a moment after mount.
async function hear(event: VoiceEvent, heard: () => void) {
  await waitFor(() => {
    backend.emit("voice", event);
    heard();
  });
}

describe("useVoice", () => {
  it("opens what an open event names", async () => {
    const { result } = mount("/settings/voice");
    await hear({ kind: "open", target: { kind: "dot", dot_id: "dot-a" } }, () =>
      expect(result.current.location.pathname).toBe("/home/dot-a"),
    );
    act(() => backend.emit("voice", { kind: "open", target: { kind: "approval", approval_id: "ap-1" } }));
    expect(result.current.location.pathname).toBe("/inbox");
    act(() => backend.emit("voice", { kind: "open", target: { kind: "usage" } }));
    expect(result.current.location.pathname).toBe("/usage");
    act(() => backend.emit("voice", { kind: "open", target: { kind: "settings" } }));
    expect(result.current.location.pathname).toBe("/settings");
    act(() => backend.emit("voice", { kind: "open", target: { kind: "inbox" } }));
    expect(result.current.location.pathname).toBe("/inbox");
    act(() => backend.emit("voice", { kind: "open", target: { kind: "home" } }));
    expect(result.current.location.pathname).toBe("/home");
    for (const kind of ["connectors", "apps", "library", "teams", "intents"] as const) {
      act(() => backend.emit("voice", { kind: "open", target: { kind } }));
      expect(result.current.location.pathname).toBe(`/${kind}`);
    }
  });

  it("reports the view on mount and again when the route changes", async () => {
    const { result, rerender } = mount("/home/dot-a");
    await waitFor(() =>
      expect(backend.callsTo("voice_view")).toEqual([
        {
          view: {
            account: sample.scope.account,
            workspace: sample.scope.workspace,
            screen: "dot",
            dot_id: "dot-a",
            topic_root: null,
          },
        },
      ]),
    );
    act(() => void result.current.navigate("/home/dot-a/run-1"));
    await waitFor(() => expect(backend.callsTo("voice_view")).toHaveLength(2));
    expect(backend.callsTo("voice_view")[1]).toEqual({
      view: {
        account: sample.scope.account,
        workspace: sample.scope.workspace,
        screen: "dot",
        dot_id: "dot-a",
        topic_root: "run-1",
      },
    });
    act(() => void result.current.navigate("/settings/voice"));
    await waitFor(() => expect(backend.callsTo("voice_view")).toHaveLength(3));
    expect(backend.callsTo("voice_view")[2]).toEqual({
      view: {
        account: sample.scope.account,
        workspace: sample.scope.workspace,
        screen: "settings",
        dot_id: null,
        topic_root: null,
      },
    });
    // A render that changes nothing sends nothing.
    vi.useFakeTimers();
    try {
      rerender();
      act(() => void vi.advanceTimersByTime(300));
      expect(backend.callsTo("voice_view")).toHaveLength(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the status in the query cache and what is heard in the store", async () => {
    const { qc } = mount("/home");
    await waitFor(() => expect(qc.getQueryData(keys.voice)).toEqual(sample.voiceOff));
    await hear({ kind: "status", status: sample.voiceReady }, () =>
      expect(qc.getQueryData(keys.voice)).toEqual(sample.voiceReady),
    );
    act(() => backend.emit("voice", { kind: "state", state: "speaking", mic_open: false }));
    expect(voiceStore.get().state).toBe("speaking");
    expect(voiceStore.get().micOpen).toBe(false);
    expect(qc.getQueryData(keys.voice)).toMatchObject({ state: "speaking", mic_open: false });
    act(() => backend.emit("voice", { kind: "level", mic: 0.5, voice: 0.25 }));
    expect(voiceStore.get().level).toEqual({ mic: 0.5, voice: 0.25 });
    act(() => backend.emit("voice", { kind: "hearing", text: "what is" }));
    expect(voiceStore.get().hearing).toBe("what is");
    act(() => backend.emit("voice", { kind: "saying", text: "Nothing waits." }));
    expect(voiceStore.get().saying).toBe("Nothing waits.");
    act(() => backend.emit("voice", { kind: "download", done_bytes: 5, total_bytes: 10 }));
    expect(voiceStore.get().download).toEqual({ done: 5, total: 10 });
    act(() => backend.emit("voice", { kind: "trouble", message: "No microphone." }));
    expect(voiceStore.get().trouble).toBe("No microphone.");
    // each trouble is counted, so the stage can open for it
    expect(voiceStore.get().troubles).toBe(1);
    act(() => backend.emit("voice", { kind: "trouble", message: "No microphone." }));
    expect(voiceStore.get().troubles).toBe(2);
    act(() => backend.emit("voice", { kind: "ask", question: "Run it?", options: ["Yes", "No"], seq: 4 }));
    expect(voiceStore.get().ask).toEqual({ question: "Run it?", options: ["Yes", "No"], seq: 4 });
    act(() => backend.emit("voice", { kind: "cleared" }));
    expect(voiceStore.get().ask).toBeNull();
    expect(voiceStore.get().hearing).toBe("");
    expect(voiceStore.get().saying).toBe("");
  });

  it("keeps a status event that arrives before the first fetch answers", async () => {
    const first = later<typeof sample.voiceOff>();
    backend.on("voice_status", () => first.answer);
    const { qc } = mount("/home");
    await hear({ kind: "status", status: sample.voiceReady }, () =>
      expect(qc.getQueryData(keys.voice)).toEqual(sample.voiceReady),
    );
    await act(async () => first.give(sample.voiceOff));
    expect(qc.getQueryData(keys.voice)).toEqual(sample.voiceReady);
    expect(voiceStore.get().state).toBe(sample.voiceReady.state);
  });

  it("clears a trouble when a status without one arrives or Open Sage listens", async () => {
    mount("/home");
    await hear({ kind: "trouble", message: "No microphone." }, () =>
      expect(voiceStore.get().trouble).toBe("No microphone."),
    );
    act(() => backend.emit("voice", { kind: "state", state: "listening", mic_open: true }));
    expect(voiceStore.get().trouble).toBeNull();
    act(() => backend.emit("voice", { kind: "trouble", message: "Again." }));
    act(() =>
      backend.emit("voice", { kind: "status", status: { ...sample.voiceReady, trouble: null } }),
    );
    expect(voiceStore.get().trouble).toBeNull();
  });

  it("forgets a reply's lines once Open Sage listens again", async () => {
    mount("/home");
    await hear({ kind: "saying", text: "Nothing needs you." }, () =>
      expect(voiceStore.get().said.map((l) => l.text)).toEqual(["Nothing needs you."]),
    );
    act(() => backend.emit("voice", { kind: "state", state: "listening", mic_open: true }));
    expect(voiceStore.get().said).toEqual([]);
    expect(voiceStore.get().saying).toBe("");
    // The listening ends in silence: nothing of the old reply comes back.
    act(() => backend.emit("voice", { kind: "state", state: "idle", mic_open: false }));
    expect(voiceStore.get().said).toEqual([]);
  });

  it("keeps the conversation: what was heard, and each reply whole in one entry", async () => {
    mount("/home");
    await hear({ kind: "heard", text: "What needs me?" }, () =>
      expect(voiceStore.get().talk.length).toBeGreaterThan(0),
    );
    act(() => voiceStore.set({ talk: voiceStore.get().talk.slice(-1) }));
    for (const text of ["Two things.", "An approval.", "And a question."]) {
      act(() => backend.emit("voice", { kind: "saying", text }));
    }
    // Listening again and the next sentence: the reply before it stays in the conversation.
    act(() => backend.emit("voice", { kind: "state", state: "listening", mic_open: true }));
    act(() => backend.emit("voice", { kind: "heard", text: "Read the question." }));
    act(() => backend.emit("voice", { kind: "saying", text: "It asks which server." }));
    expect(voiceStore.get().talk.map((line) => [line.who, line.text])).toEqual([
      ["you", "What needs me?"],
      ["sage", "Two things. An approval. And a question."],
      ["you", "Read the question."],
      ["sage", "It asks which server."],
    ]);
    const numbers = voiceStore.get().talk.map((line) => line.n);
    expect(new Set(numbers).size).toBe(numbers.length);
  });

  it("keeps the steps Sage takes between what was heard and its reply", async () => {
    mount("/home");
    await hear({ kind: "heard", text: "Open the inbox." }, () =>
      expect(voiceStore.get().talk.length).toBeGreaterThan(0),
    );
    act(() => voiceStore.set({ talk: voiceStore.get().talk.slice(-1) }));
    act(() => backend.emit("voice", { kind: "state", state: "thinking", mic_open: false }));
    expect(voiceStore.get().step).toBeNull();
    act(() => backend.emit("voice", { kind: "step", text: "Looking at your workspace" }));
    expect(voiceStore.get().step).toBe("Looking at your workspace");
    // The same step again is one step; an empty one is none.
    act(() => backend.emit("voice", { kind: "step", text: "Looking at your workspace" }));
    act(() => backend.emit("voice", { kind: "step", text: "  " }));
    act(() => backend.emit("voice", { kind: "step", text: "Opening the Inbox" }));
    expect(voiceStore.get().step).toBe("Opening the Inbox");
    // It speaks from the first words of the reply: the step goes on until the core says it
    // is over, and the steps stay in the conversation.
    act(() => backend.emit("voice", { kind: "state", state: "speaking", mic_open: false }));
    expect(voiceStore.get().step).toBe("Opening the Inbox");
    act(() => backend.emit("voice", { kind: "step", text: "" }));
    expect(voiceStore.get().step).toBeNull();
    act(() => backend.emit("voice", { kind: "saying", text: "The Inbox is open." }));
    expect(voiceStore.get().talk.map((line) => [line.who, line.text])).toEqual([
      ["you", "Open the inbox."],
      ["step", "Looking at your workspace"],
      ["step", "Opening the Inbox"],
      ["sage", "The Inbox is open."],
    ]);
    // The next sentence heard, and a step that was dropped, leave none under way.
    act(() => backend.emit("voice", { kind: "state", state: "thinking", mic_open: false }));
    act(() => backend.emit("voice", { kind: "step", text: "Thinking it over" }));
    act(() => backend.emit("voice", { kind: "heard", text: "And usage." }));
    expect(voiceStore.get().step).toBeNull();
    act(() => backend.emit("voice", { kind: "step", text: "Thinking it over" }));
    act(() => backend.emit("voice", { kind: "cleared" }));
    expect(voiceStore.get().step).toBeNull();
  });

  it("keeps the steps of a turn together above its reply, and the reply in one entry", async () => {
    mount("/home");
    await hear({ kind: "heard", text: "Tell research hello." }, () =>
      expect(voiceStore.get().talk.length).toBeGreaterThan(0),
    );
    const talk = () => voiceStore.get().talk.map((line) => [line.who, line.text]);
    act(() => backend.emit("voice", { kind: "state", state: "thinking", mic_open: false }));
    act(() => backend.emit("voice", { kind: "step", text: "Thinking it over" }));
    // A line said while it still thinks is not the reply: the step goes on.
    act(() => backend.emit("voice", { kind: "saying", text: "One moment." }));
    expect(voiceStore.get().step).toBe("Thinking it over");
    // The reply has begun when the deed is named: the step is under way while it speaks.
    act(() => backend.emit("voice", { kind: "state", state: "speaking", mic_open: false }));
    act(() => backend.emit("voice", { kind: "step", text: "Sending your message to Research" }));
    // A line written before the step is spoken, and so shown, after it: the step goes on.
    act(() => backend.emit("voice", { kind: "saying", text: "Sending." }));
    expect(voiceStore.get().step).toBe("Sending your message to Research");
    act(() => backend.emit("voice", { kind: "step", text: "" }));
    expect(voiceStore.get().step).toBeNull();
    act(() => backend.emit("voice", { kind: "saying", text: "Sent to Research." }));
    expect(talk()).toEqual([
      ["you", "Tell research hello."],
      ["step", "Thinking it over"],
      ["step", "Sending your message to Research"],
      ["sage", "One moment. Sending. Sent to Research."],
    ]);
    // The turn ends with a step still shown: none is under way.
    act(() => backend.emit("voice", { kind: "step", text: "Opening Research" }));
    act(() => backend.emit("voice", { kind: "state", state: "listening", mic_open: true }));
    expect(voiceStore.get().step).toBeNull();
    // A step that crossed the end of the turn on its way is not shown at all.
    const before = talk();
    act(() => backend.emit("voice", { kind: "step", text: "Too late" }));
    expect(voiceStore.get().step).toBeNull();
    expect(talk()).toEqual(before);
  });

  it("drops the steps taken for the first part of a sentence the person went on with", async () => {
    mount("/home");
    await hear({ kind: "heard", text: "Open the" }, () =>
      expect(voiceStore.get().talk.length).toBeGreaterThan(0),
    );
    act(() => backend.emit("voice", { kind: "state", state: "thinking", mic_open: false }));
    act(() => backend.emit("voice", { kind: "step", text: "Looking at your workspace" }));
    act(() => backend.emit("voice", { kind: "step", text: "Thinking it over" }));
    act(() => backend.emit("voice", { kind: "heard", text: "Open the inbox." }));
    expect(voiceStore.get().talk.map((line) => [line.who, line.text])).toEqual([
      ["you", "Open the inbox."],
    ]);
    // The turn over with only steps shown (no speaker to say the reply), the same opening
    // words are a new sentence: the steps before it stay.
    act(() => backend.emit("voice", { kind: "state", state: "thinking", mic_open: false }));
    act(() => backend.emit("voice", { kind: "step", text: "Opening the Inbox" }));
    act(() => backend.emit("voice", { kind: "state", state: "idle", mic_open: false }));
    act(() => backend.emit("voice", { kind: "heard", text: "Open the inbox. Again." }));
    expect(voiceStore.get().talk.map((line) => [line.who, line.text])).toEqual([
      ["you", "Open the inbox."],
      ["step", "Opening the Inbox"],
      ["you", "Open the inbox. Again."],
    ]);
  });

  it("keeps the last sixty things said, however many steps lie between them", async () => {
    mount("/home");
    await hear({ kind: "heard", text: "Sentence 0." }, () =>
      expect(voiceStore.get().talk.length).toBeGreaterThan(0),
    );
    for (let i = 1; i <= 70; i += 1) {
      act(() => backend.emit("voice", { kind: "step", text: `Step ${i} a` }));
      act(() => backend.emit("voice", { kind: "step", text: `Step ${i} b` }));
      act(() => backend.emit("voice", { kind: "heard", text: `Sentence ${i}.` }));
    }
    const talk = voiceStore.get().talk;
    const said = talk.filter((line) => line.who !== "step");
    expect(said).toHaveLength(60);
    expect(said[0]!.text).toBe("Sentence 11.");
    expect(talk[0]!.text).toBe("Sentence 11.");
    expect(talk.filter((line) => line.who === "step")).toHaveLength(118);
  });

  it("wakes no reader for a level that changes nothing", async () => {
    mount("/home");
    await hear({ kind: "level", mic: 0.5, voice: 0 }, () =>
      expect(voiceStore.get().level.mic).toBe(0.5),
    );
    let woken = 0;
    const stop = voiceStore.subscribe(() => (woken += 1));
    act(() => backend.emit("voice", { kind: "level", mic: 0.5, voice: 0 }));
    expect(woken).toBe(0);
    act(() => backend.emit("voice", { kind: "level", mic: 0.25, voice: 0 }));
    expect(woken).toBe(1);
    stop();
  });

  it("keeps the last three lines of a reply, and starts afresh on what is heard next", async () => {
    mount("/home");
    await hear({ kind: "hearing", text: "what is" }, () =>
      expect(voiceStore.get().hearing).toBe("what is"),
    );
    // Still being heard: not finished.
    expect(voiceStore.get().heard).toBe(false);
    act(() => backend.emit("voice", { kind: "heard", text: "what is waiting" }));
    expect(voiceStore.get().hearing).toBe("what is waiting");
    expect(voiceStore.get().heard).toBe(true);
    act(() => backend.emit("voice", { kind: "saying", text: "One." }));
    // A reply starts: what was heard is done with.
    expect(voiceStore.get().hearing).toBe("");
    expect(voiceStore.get().heard).toBe(false);
    const texts = () => voiceStore.get().said.map((l) => l.text);
    const numbers = () => voiceStore.get().said.map((l) => l.n);
    expect(texts()).toEqual(["One."]);
    for (const line of ["Two.", "Three.", "Four."]) {
      act(() => backend.emit("voice", { kind: "saying", text: line }));
    }
    expect(texts()).toEqual(["Two.", "Three.", "Four."]);
    // Each line keeps its own running number, so a held line is never named as another.
    const [a, b, c] = numbers();
    expect(b).toBe(a! + 1);
    expect(c).toBe(b! + 1);
    expect(voiceStore.get().saying).toBe("Four.");
    act(() => backend.emit("voice", { kind: "heard", text: "and then" }));
    expect(voiceStore.get().said).toEqual([]);
    act(() => backend.emit("voice", { kind: "saying", text: "Then." }));
    // The count runs on across replies.
    expect(numbers()[0]).toBeGreaterThan(c!);
    act(() => backend.emit("voice", { kind: "cleared" }));
    expect(voiceStore.get().said).toEqual([]);
    expect(voiceStore.get().heard).toBe(false);
  });

  it("keeps a status event when a screen's read of the status was on its way", async () => {
    const reads: ReturnType<typeof later<typeof sample.voiceOff>>[] = [];
    backend.on("voice_status", () => {
      const read = later<typeof sample.voiceOff>();
      reads.push(read);
      return read.answer;
    });
    const qc = createQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: qc },
        createElement(MemoryRouter, { initialEntries: ["/home"] }, children),
      );
    const { result } = renderHook(
      () => {
        useVoice();
        return useVoiceStatus();
      },
      { wrapper },
    );
    // One question only: the screen's reader and the frame share it.
    await waitFor(() => expect(reads).toHaveLength(1));
    await hear({ kind: "status", status: sample.voiceReady }, () =>
      expect(result.current.data).toEqual(sample.voiceReady),
    );
    await act(async () => reads[0]!.give(sample.voiceOff));
    expect(qc.getQueryData(keys.voice)).toEqual(sample.voiceReady);
    expect(result.current.data).toEqual(sample.voiceReady);
    expect(backend.callsTo("voice_status")).toHaveLength(1);
  });

  it("stops listening when it unmounts", async () => {
    const { unmount } = mount("/home");
    await hear({ kind: "hearing", text: "one" }, () =>
      expect(voiceStore.get().hearing).toBe("one"),
    );
    unmount();
    backend.emit("voice", { kind: "hearing", text: "two" });
    expect(voiceStore.get().hearing).toBe("one");
  });
});
