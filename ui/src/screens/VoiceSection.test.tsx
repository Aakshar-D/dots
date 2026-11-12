import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Profiler, useState } from "react";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import type { Settings, VoiceStatus } from "@/api/types";
import { createQueryClient } from "@/lib/query";
import { useVoice } from "@/lib/use-voice";
import { VoiceSection } from "@/screens/VoiceSection";
import { backend, later } from "@/test/backend";
import { sample } from "@/test/fixtures";
import { renderApp } from "@/test/render";

const ready: VoiceStatus = { ...sample.voiceReady, trouble: null, hotkey_error: null };
const devices = {
  microphones: [
    { id: "mic-laptop", name: "Laptop microphone", default: true },
    { id: "mic-usb", name: "USB microphone", default: false },
  ],
  speakers: [{ id: "spk-laptop", name: "Laptop speakers", default: true }],
};

/// A backend whose settings are `stored` and whose voice answers `status`; turning voice on
/// saves the setting, as the core does.
function voice(status: VoiceStatus, stored: Settings = sample.settings) {
  const now = { settings: stored, status };
  backend.on("get_settings", () => now.settings);
  backend.on("update_settings", (args) => {
    now.settings = args.settings as Settings;
    return { settings: now.settings, restart_required: false };
  });
  backend.on("voice_status", () => now.status);
  backend.on("voice_devices", () => devices);
  backend.on("voice_preview", () => null);
  backend.on("voice_download_cancel", () => null);
  backend.on("voice_enable", (args) => {
    now.settings = { ...now.settings, voice_enabled: args.on as boolean };
    now.status = args.on ? ready : { ...status, enabled: false, ready: false, state: "off" };
    return now.status;
  });
  backend.on("voice_remove", () => {
    now.settings = { ...now.settings, voice_enabled: false };
    now.status = sample.voiceOff;
    return now.status;
  });
  return now;
}

describe("Settings > Voice", () => {
  it("is off at first and explains the download", async () => {
    voice(sample.voiceOff);
    renderApp("/settings/voice");
    const toggle = await screen.findByRole("switch", { name: "Open Dot" });
    expect(toggle).not.toBeChecked();
    expect(screen.getByText("Open Dot lets you talk to Dots and hear it answer.")).toBeVisible();
    expect(
      screen.getByText(
        "Turning it on downloads about 260 MB of speech files once; they stay on this computer.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("status", { name: "Open Dot status" })).toHaveTextContent("Off");
    // What needs the session waits for it; what only stores a value does not.
    expect(screen.getByRole("button", { name: "Play voice 1" })).toBeDisabled();
    expect(screen.getByLabelText("Microphone")).toBeDisabled();
    // Said beside the voices and beside the devices.
    expect(
      screen.getAllByText("Turn Open Dot on to try the voices and choose devices."),
    ).toHaveLength(2);
    expect(screen.getByLabelText("Hotkey")).toBeEnabled();
    expect(screen.getByRole("switch", { name: "Hey Dot" })).toBeEnabled();
    expect(screen.getByLabelText("Which model thinks")).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Remove speech files" })).toBeNull();
  });

  it("turns on, shows the download's progress, and cancels it", async () => {
    voice(sample.voiceOff);
    const download = later<VoiceStatus>();
    backend.on("voice_enable", () => download.answer);
    const { user, listening } = renderApp("/settings/voice");
    await listening();
    await user.click(await screen.findByRole("switch", { name: "Open Dot" }));
    expect(backend.callsTo("voice_enable")).toEqual([{ on: true }]);
    act(() =>
      backend.emit("voice", { kind: "download", done_bytes: 52_000_000, total_bytes: 258_000_000 }),
    );
    const bar = await screen.findByRole("progressbar", { name: "Speech files downloaded" });
    expect(bar).toHaveAttribute("aria-valuenow", "52000000");
    expect(bar).toHaveAttribute("aria-valuemax", "258000000");
    expect(screen.getByText("52 MB of 258 MB")).toBeVisible();
    expect(screen.getByRole("status", { name: "Open Dot status" })).toHaveTextContent(
      "Getting ready",
    );

    await user.click(screen.getByRole("button", { name: "Cancel download" }));
    expect(backend.callsTo("voice_download_cancel")).toEqual([{}]);
    act(() => download.fail({ kind: "cancelled", message: "the download was cancelled" }));
    await waitFor(() => expect(screen.queryByRole("progressbar")).toBeNull());
    expect(screen.getByRole("switch", { name: "Open Dot" })).not.toBeChecked();
  });

  it("shows why turning on failed", async () => {
    voice(sample.voiceOff);
    backend.on("voice_enable", () => {
      throw { kind: "internal", message: "The speech files could not be downloaded." };
    });
    const { user } = renderApp("/settings/voice");
    await user.click(await screen.findByRole("switch", { name: "Open Dot" }));
    expect(await screen.findByText("The speech files could not be downloaded.")).toBeVisible();
    expect(screen.getByRole("switch", { name: "Open Dot" })).not.toBeChecked();
  });

  it("stays on when another field is saved after turning on", async () => {
    voice(sample.voiceOff);
    const { user } = renderApp("/settings/voice");
    await user.click(await screen.findByRole("switch", { name: "Open Dot" }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "Open Dot" })).toBeChecked());
    // The switch acts at once: nothing waits for Save.
    expect(screen.queryByText("Unsaved changes")).toBeNull();
    await user.click(screen.getByRole("switch", { name: "Hey Dot" }));
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_settings")).toHaveLength(1));
    const saved = backend.callsTo("update_settings")[0].settings as Settings;
    expect(saved.voice_enabled).toBe(true);
    expect(saved.voice_wake_phrase).toBe(true);
  });

  it("turns the listening tone off through the form", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true, voice_tone: true });
    const { user } = renderApp("/settings/voice");
    const tone = await screen.findByRole("switch", { name: "Play a tone when Dot starts listening" });
    expect(tone).toBeChecked();
    expect(tone).toBeEnabled();
    await user.click(tone);
    expect(tone).not.toBeChecked();
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(backend.callsTo("update_settings")).toHaveLength(1));
    const saved = backend.callsTo("update_settings")[0].settings as Settings;
    expect(saved.voice_tone).toBe(false);
  });

  it("shows the controls when ready, and plays a voice at the chosen speed", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true });
    const { user } = renderApp("/settings/voice");
    expect(await screen.findByRole("switch", { name: "Open Dot" })).toBeChecked();
    const status = screen.getByRole("status", { name: "Open Dot status" });
    expect(status).toHaveTextContent("Ready");
    expect(status).toHaveTextContent("Listening");
    const voices = screen.getByRole("radiogroup", { name: "Voice" });
    expect(within(voices).getAllByRole("radio")).toHaveLength(10);
    expect(within(voices).getByRole("radio", { name: "Voice 1" })).toBeChecked();

    await user.click(screen.getByRole("button", { name: "Play voice 3" }));
    expect(backend.callsTo("voice_preview")).toEqual([{ voice: 2, speed: 1 }]);

    const speed = screen.getByRole("slider", { name: "Speaking speed" });
    fireEvent.change(speed, { target: { value: "1.15" } });
    expect(screen.getByText("1.15×")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Play voice 10" }));
    expect(backend.callsTo("voice_preview")[1]).toEqual({ voice: 9, speed: 1.15 });
  });

  it("says why a preview could not play", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true });
    backend.on("voice_preview", () => {
      throw { kind: "internal", message: "Open Dot is not ready yet." };
    });
    const { user } = renderApp("/settings/voice");
    await user.click(await screen.findByRole("button", { name: "Play voice 2" }));
    expect(await screen.findByText("Open Dot is not ready yet.")).toBeVisible();
  });

  it("shows the hotkey's error, and Clear empties the hotkey", async () => {
    voice({ ...ready, hotkey_error: "Another app uses Ctrl+Shift+Space." });
    const { user } = renderApp("/settings/voice");
    expect(await screen.findByText("Another app uses Ctrl+Shift+Space.")).toBeVisible();
    const hotkey = screen.getByLabelText("Hotkey");
    // The stored "CmdOrCtrl+Shift+Space", as this computer names the keys.
    expect(hotkey).toHaveValue("Ctrl+Shift+Space");
    expect(screen.getByText("Use names like Ctrl, Shift, Alt and a key, joined with +.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Clear the hotkey" }));
    expect(hotkey).toHaveValue("");
    expect(screen.getByText("Unsaved changes")).toBeVisible();
  });

  it("saves a new microphone through the form", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true });
    const { user } = renderApp("/settings/voice");
    const mic = await screen.findByLabelText("Microphone");
    await waitFor(() => expect(within(mic).getAllByRole("option")).toHaveLength(3));
    expect(within(mic).getAllByRole("option")[0]).toHaveTextContent("System default");
    await user.selectOptions(mic, "mic-usb");
    expect(screen.getByText("Unsaved changes")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, voice_enabled: true, voice_mic: "mic-usb" } },
      ]),
    );
  });

  it("binds the model to open_dot_model", async () => {
    voice(sample.voiceOff);
    const { user } = renderApp("/settings/voice");
    const model = await screen.findByLabelText("Which model thinks");
    expect(within(model).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Fast",
      "Balanced",
      "Most capable",
    ]);
    await user.selectOptions(model, "opus");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.callsTo("update_settings")).toEqual([
        { settings: { ...sample.settings, open_dot_model: "opus" } },
      ]),
    );
  });

  it("asks before it removes the speech files", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true });
    const { user } = renderApp("/settings/voice");
    await user.click(await screen.findByRole("button", { name: "Remove speech files" }));
    const dialog = await screen.findByRole("alertdialog");
    expect(backend.callsTo("voice_remove")).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(backend.callsTo("voice_remove")).toEqual([{}]));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Open Dot" })).not.toBeChecked(),
    );
  });

  it("shows a download it did not start, and cannot start a second", async () => {
    voice({ ...sample.voiceOff, enabled: false, downloading: true, download_bytes: 258_000_000 });
    const { user, listening } = renderApp("/settings/voice");
    await listening();
    const toggle = await screen.findByRole("switch", { name: "Open Dot" });
    expect(toggle).toBeChecked();
    expect(toggle).toHaveAttribute("aria-disabled", "true");
    const bar = await screen.findByRole("progressbar", { name: "Speech files downloaded" });
    expect(bar).not.toHaveAttribute("aria-valuenow");
    expect(screen.getByText("Getting the speech files…")).toBeVisible();
    expect(screen.queryByText(/0 MB/)).toBeNull();
    await user.click(toggle);
    expect(backend.callsTo("voice_enable")).toEqual([]);
    act(() =>
      backend.emit("voice", { kind: "download", done_bytes: 52_000_000, total_bytes: 258_000_000 }),
    );
    expect(await screen.findByText("52 MB of 258 MB")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel download" }));
    expect(backend.callsTo("voice_download_cancel")).toEqual([{}]);
  });

  it("gives focus back to the switch when the download ends under Cancel", async () => {
    voice({ ...sample.voiceOff, enabled: false, downloading: true, download_bytes: 258_000_000 });
    const { user, listening } = renderApp("/settings/voice");
    await listening();
    await user.click(await screen.findByRole("button", { name: "Cancel download" }));
    act(() => backend.emit("voice", { kind: "status", status: ready }));
    await waitFor(() => expect(screen.queryByRole("progressbar")).toBeNull());
    expect(screen.getByRole("switch", { name: "Open Dot" })).toHaveFocus();
  });

  it("shows a saved device as a saved device until the list has loaded", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true, voice_mic: "mic-usb" });
    const list = later<typeof devices>();
    backend.on("voice_devices", () => list.answer);
    renderApp("/settings/voice");
    const mic = await screen.findByLabelText("Microphone");
    await waitFor(() => expect(mic).toHaveDisplayValue("Saved device"));
    expect(screen.queryByText(/mic-usb/)).toBeNull();
    act(() => list.give(devices));
    await waitFor(() => expect(mic).toHaveDisplayValue("USB microphone"));
  });

  it("shows a saved device that is in the list by its name", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true, voice_speaker: "spk-laptop" });
    renderApp("/settings/voice");
    const speaker = await screen.findByLabelText("Speaker");
    await waitFor(() => expect(speaker).toHaveDisplayValue("Laptop speakers"));
  });

  it("says a saved device is not connected only once the list has loaded", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true, voice_mic: "gone-id" });
    renderApp("/settings/voice");
    const mic = await screen.findByLabelText("Microphone");
    await waitFor(() => expect(mic).toHaveDisplayValue("Saved device (not connected)"));
    expect(screen.queryByText(/gone-id/)).toBeNull();
  });

  it("lists the devices as soon as voice is on, before it is ready", async () => {
    voice({ ...ready, ready: false, state: "off" }, { ...sample.settings, voice_enabled: true });
    renderApp("/settings/voice");
    await waitFor(() => expect(backend.callsTo("voice_devices").length).toBeGreaterThan(0));
  });

  it("shows a trouble event in the status line at once", async () => {
    voice(ready, { ...sample.settings, voice_enabled: true });
    const { listening } = renderApp("/settings/voice");
    await listening();
    await screen.findByText("Ready");
    act(() => backend.emit("voice", { kind: "trouble", message: "No microphone found." }));
    expect(screen.getByRole("status", { name: "Open Dot status" })).toHaveTextContent(
      "No microphone found.",
    );
    act(() => backend.emit("voice", { kind: "state", state: "listening", mic_open: true }));
    expect(screen.getByRole("status", { name: "Open Dot status" })).toHaveTextContent("Ready");
  });

  it("asks for the voice status again after a save, so a fixed hotkey clears its error", async () => {
    const now = voice(
      { ...ready, hotkey_error: "Another app uses Ctrl+Shift+Space." },
      { ...sample.settings, voice_enabled: true },
    );
    const { user } = renderApp("/settings/voice");
    expect(await screen.findByText("Another app uses Ctrl+Shift+Space.")).toBeVisible();
    now.status = { ...ready, hotkey_error: null };
    await user.type(screen.getByLabelText("Hotkey"), "x");
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(screen.queryByText("Another app uses Ctrl+Shift+Space.")).toBeNull(),
    );
  });

  it("follows a status event without asking again", async () => {
    voice(sample.voiceOff);
    const { listening } = renderApp("/settings/voice");
    await listening();
    await screen.findByRole("switch", { name: "Open Dot" });
    const asked = backend.callsTo("voice_status").length;
    act(() => backend.emit("voice", { kind: "status", status: ready }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "Open Dot" })).toBeChecked());
    expect(screen.getByRole("status", { name: "Open Dot status" })).toHaveTextContent("Ready");
    expect(backend.callsTo("voice_status")).toHaveLength(asked);
  });
});

describe("VoiceSection", () => {
  it("does not draw again for a level", async () => {
    backend.on("voice_status", () => ready);
    backend.on("voice_devices", () => devices);
    let renders = 0;
    function Hook() {
      useVoice();
      return null;
    }
    function Tab() {
      const [draft, setDraft] = useState(sample.settings);
      return <VoiceSection draft={draft} set={(c) => setDraft({ ...draft, ...c })} />;
    }
    render(
      <QueryClientProvider client={createQueryClient()}>
        <MemoryRouter initialEntries={["/settings/voice"]}>
          <Hook />
          <Profiler id="voice" onRender={() => (renders += 1)}>
            <Tab />
          </Profiler>
        </MemoryRouter>
      </QueryClientProvider>,
    );
    await screen.findByText("Ready");
    // The listener attaches a moment after mount: a state event shows when it has.
    await waitFor(() => {
      backend.emit("voice", { kind: "state", state: "listening", mic_open: false });
      expect(screen.getByRole("status", { name: "Open Dot status" })).not.toHaveTextContent(
        "Listening",
      );
    });
    await waitFor(() => expect(within(screen.getByLabelText("Microphone")).getAllByRole("option")).toHaveLength(3));
    const before = renders;
    act(() => {
      for (let i = 0; i < 30; i++) backend.emit("voice", { kind: "level", mic: i / 30, voice: 0 });
    });
    expect(renders).toBe(before);
  });
});
