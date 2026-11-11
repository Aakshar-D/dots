import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { VoiceStatus } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { voiceStore, type VoiceLive } from "@/lib/use-voice";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { RailMark } from "./RailMark";

const ready: VoiceStatus = { ...sample.voiceReady, trouble: null, mic_open: false, state: "idle" };

/// The mark at /home, with a settings page to go to.
function mount(status: VoiceStatus | "never") {
  backend.on("voice_status", () => (status === "never" ? new Promise(() => {}) : status));
  const router = createMemoryRouter(
    [
      { path: "/home", element: <RailMark /> },
      { path: "/settings/:tab", element: <p>Settings page</p> },
    ],
    { initialEntries: ["/home"] },
  );
  render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  return router;
}

const mark = () => screen.getByRole("button", { name: /^(Talk to Dot|Set up voice)$/ });

beforeEach(() => {
  voiceStore.reset();
  backend.on("voice_open", () => null);
  backend.on("voice_stop", () => null);
});
afterEach(() => voiceStore.reset());

describe("the rail mark", () => {
  it("listens when voice is ready, and gives the shortcut apart from its name", async () => {
    backend.on("get_settings", () => ({ ...sample.settings, voice_hotkey: "CmdOrCtrl+Shift+Space" }));
    mount(ready);
    await waitFor(() => expect(mark()).toHaveAttribute("aria-keyshortcuts", "Control+Shift+Space"));
    expect(mark()).toHaveAccessibleName("Talk to Dot");
    expect(mark()).toHaveAttribute("title", "Talk to Dot (Ctrl+Shift+Space)");
    fireEvent.click(mark());
    expect(backend.callsTo("voice_open")).toEqual([{}]);
  });

  it("names no shortcut when none is set", async () => {
    backend.on("get_settings", () => ({ ...sample.settings, voice_hotkey: "" }));
    mount(ready);
    await waitFor(() => expect(backend.callsTo("get_settings")).toHaveLength(1));
    await act(async () => {});
    expect(mark()).not.toHaveAttribute("aria-keyshortcuts");
    expect(mark()).toHaveAttribute("title", "Talk to Dot");
  });

  it.each<[string, Partial<VoiceLive>, string]>([
    ["thinking", { state: "thinking" }, "voice_stop"],
    ["speaking", { state: "speaking" }, "voice_stop"],
    ["listening", { state: "listening", micOpen: true }, "voice_open"],
    ["idle with a question waiting", { state: "idle", ask: { question: "Run it?", options: ["Yes", "No"], seq: 1 } }, "voice_open"],
    ["idle with a card showing", { state: "idle", card: { kind: "list", title: "Waiting", lines: [] } }, "voice_open"],
  ])("while Open Dot is %s, a click calls %s", async (_, live, command) => {
    mount(ready);
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    await act(async () => {});
    act(() => voiceStore.set(live));
    fireEvent.click(mark());
    expect(backend.callsTo(command)).toEqual([{}]);
    expect(backend.callsTo(command === "voice_stop" ? "voice_open" : "voice_stop")).toEqual([]);
  });

  it("does nothing while the voice status is still on its way", async () => {
    const router = mount("never");
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    fireEvent.click(mark());
    await act(async () => {});
    expect(router.state.location.pathname).toBe("/home");
    expect(backend.callsTo("voice_open")).toEqual([]);
    expect(backend.callsTo("voice_stop")).toEqual([]);
  });

  it.each<[string, VoiceStatus]>([
    ["off", sample.voiceOff],
    ["not installed", { ...ready, enabled: true, installed: false, ready: false }],
    ["still getting ready", { ...ready, ready: false }],
  ])("goes to the voice settings when voice is %s", async (_, status) => {
    const router = mount(status);
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    await act(async () => {});
    expect(mark()).toHaveAccessibleName("Set up voice");
    fireEvent.click(mark());
    expect(await screen.findByText("Settings page")).toBeVisible();
    expect(router.state.location.pathname).toBe("/settings/voice");
    expect(backend.callsTo("voice_open")).toEqual([]);
  });

  it("shows a steady ring while the microphone is open", async () => {
    mount(ready);
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    expect(mark()).not.toHaveAttribute("data-listening");
    act(() => voiceStore.set({ state: "listening", micOpen: true }));
    expect(mark()).toHaveAttribute("data-listening");
    expect(mark()).toHaveClass("ring-primary");
    expect(mark().className).not.toMatch(/animate-/);
    expect(mark()).toHaveAttribute("title", "Dot is listening");
  });
});
