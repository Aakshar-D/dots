import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createMemoryRouter, RouterProvider } from "react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { VoiceStatus } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { voiceStore } from "@/lib/use-voice";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { RailMark } from "./RailMark";

const ready: VoiceStatus = { ...sample.voiceReady, trouble: null, mic_open: false, state: "idle" };

/// The mark at /home, with a settings page to go to.
function mount(status: VoiceStatus) {
  backend.on("voice_status", () => status);
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

const mark = () => screen.getByRole("button", { name: /^Talk to Dot/ });

beforeEach(() => {
  voiceStore.reset();
  backend.on("voice_open", () => null);
  backend.on("voice_stop", () => null);
});
afterEach(() => voiceStore.reset());

describe("the rail mark", () => {
  it("listens when voice is ready, and names the shortcut", async () => {
    mount(ready);
    await waitFor(() => expect(mark()).toHaveAccessibleName("Talk to Dot (Ctrl+Shift+Space)"));
    fireEvent.click(mark());
    expect(backend.callsTo("voice_open")).toEqual([{}]);
  });

  it("stops and listens while Open Dot is busy, as the shortcut does", async () => {
    mount(ready);
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    act(() => voiceStore.set({ state: "speaking" }));
    await waitFor(() => {
      fireEvent.click(mark());
      expect(backend.callsTo("voice_stop")).toHaveLength(1);
    });
    expect(backend.callsTo("voice_open")).toEqual([]);
  });

  it.each<[string, VoiceStatus]>([
    ["off", sample.voiceOff],
    ["not installed", { ...ready, enabled: true, installed: false, ready: false }],
    ["still getting ready", { ...ready, ready: false }],
  ])("goes to the voice settings when voice is %s", async (_, status) => {
    const router = mount(status);
    await waitFor(() => expect(backend.callsTo("voice_status")).toHaveLength(1));
    await act(async () => {});
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
