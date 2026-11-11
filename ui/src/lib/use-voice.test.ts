import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { MemoryRouter, useLocation, useNavigate } from "react-router";
import { describe, expect, it } from "vitest";

import type { VoiceEvent } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { useVoice, voiceStore } from "@/lib/use-voice";
import { backend } from "@/test/backend";
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
  });

  it("reports the view on mount and again when the route changes", async () => {
    const { result } = mount("/home/dot-a");
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
    await new Promise((r) => setTimeout(r, 300));
    expect(backend.callsTo("voice_view")).toHaveLength(3);
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
    act(() => backend.emit("voice", { kind: "cleared" }));
    expect(voiceStore.get().hearing).toBe("");
    expect(voiceStore.get().saying).toBe("");
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
