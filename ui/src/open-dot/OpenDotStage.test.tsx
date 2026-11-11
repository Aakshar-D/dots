import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Profiler } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenDotCard, SetupProposal } from "@/api/types";
import { voiceStore, type VoiceLive } from "@/lib/use-voice";
import { backend } from "@/test/backend";

import { DONE_MS } from "./breath-model";
import { OpenDotStage } from "./OpenDotStage";
import { LINGER_MS } from "./use-stage";

const show = (change: Partial<VoiceLive>) => act(() => voiceStore.set(change));
const stage = () => screen.queryByRole("region", { name: "Open Dot" });
const words = () => stage()!.querySelector<HTMLElement>("[aria-live='polite']")!;
/// What The Breath shows: its button names the state only through the image underneath, so
/// the state is read from the mark's data attribute.
const breathState = () => stage()!.querySelector("[data-breath-state]")?.getAttribute("data-breath-state");

const proposal: SetupProposal = {
  name: "Downloads tidy",
  instructions: "Read the Downloads folder.\nSort what is there.",
  folder: "C:\\Users\\me\\Downloads",
  schedule: {
    rule: { kind: "weekly", days: ["mon", "tue", "wed", "thu", "fri"], time: "09:00" },
    catch_up: false,
    until: null,
  },
  preset: "read-only",
  extras: [],
  uses: { mcp_servers: [], skills: [], plugins: [], context: [], apps: [] },
  first_message: "Start with this week's files.",
  notes: "",
};

/// A `prefers-reduced-motion` answer for the stage and The Breath.
function reducedMotion(reduce: boolean) {
  window.matchMedia = ((media: string) => ({
    matches: media.includes("reduce") ? reduce : false,
    media,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  })) as typeof window.matchMedia;
}
const originalMatchMedia = window.matchMedia;

beforeEach(() => {
  voiceStore.reset();
  for (const cmd of ["voice_close", "voice_stop", "voice_pick", "voice_open"]) {
    backend.on(cmd, () => null);
  }
});
afterEach(() => {
  vi.useRealTimers();
  window.matchMedia = originalMatchMedia;
  voiceStore.reset();
});

describe("the Open Dot stage", () => {
  it("does not render at all while closed", () => {
    show({ state: "idle" });
    const { container } = render(<OpenDotStage />);
    expect(stage()).toBeNull();
    expect(container.querySelector("svg")).toBeNull();
  });

  it("opens on listening and shows what it hears", () => {
    show({ state: "idle" });
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true, hearing: "what is waiting" });
    expect(stage()).toBeVisible();
    expect(words()).toHaveTextContent("what is waiting");
    expect(within(words()).getByText("what is waiting")).toHaveClass("text-muted-foreground");
    // Focus stays where it was.
    expect(document.activeElement).toBe(document.body);
  });

  it("shows the reply's two newest lines, the newest strongest", () => {
    render(<OpenDotStage />);
    show({
      state: "speaking",
      micOpen: false,
      hearing: "",
      said: ["First.", "Second.", "Third and newest."],
    });
    expect(words()).not.toHaveTextContent("First.");
    const older = within(words()).getByText("Second.");
    const newest = within(words()).getByText("Third and newest.");
    expect(older).toHaveClass("text-muted-foreground");
    expect(newest).toHaveClass("text-foreground");
    expect(newest).toHaveAttribute("title", "Third and newest.");
  });

  it("asks with numbered options, and a click picks one once", () => {
    render(<OpenDotStage />);
    show({ state: "asking", micOpen: true, ask: { question: "Run it now?", options: ["Yes", "Not now"] } });
    const group = screen.getByRole("group", { name: "Run it now?" });
    const options = within(group).getAllByRole("button");
    expect(options.map((o) => o.textContent)).toEqual(["1Yes", "2Not now"]);
    fireEvent.click(within(group).getByRole("button", { name: "Not now" }));
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1 }]);
    // A second click while that pick is on its way does nothing.
    fireEvent.click(within(group).getByRole("button", { name: "Yes" }));
    fireEvent.keyDown(document.body, { key: "1" });
    expect(backend.callsTo("voice_pick")).toHaveLength(1);
    // The next question can be answered again.
    show({ ask: { question: "Which one?", options: ["A", "B"] } });
    fireEvent.click(screen.getByRole("button", { name: "A" }));
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1 }, { index: 0 }]);
  });

  it("number keys pick, unless typing or beyond the options", () => {
    render(
      <>
        <input aria-label="Notes" />
        <OpenDotStage />
      </>,
    );
    show({ state: "asking", micOpen: true, ask: { question: "Which?", options: ["A", "B"] } });
    const input = screen.getByRole("textbox", { name: "Notes" });
    input.focus();
    fireEvent.keyDown(input, { key: "2" });
    expect(backend.callsTo("voice_pick")).toEqual([]);
    input.blur();
    fireEvent.keyDown(document.body, { key: "3" });
    expect(backend.callsTo("voice_pick")).toEqual([]);
    fireEvent.keyDown(document.body, { key: "2" });
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1 }]);
  });

  it("shows key hints for the first four of more options, and wraps the rest", () => {
    render(<OpenDotStage />);
    const options = ["One", "Two", "Three", "Four", "Five", "Six"];
    show({ state: "asking", micOpen: true, ask: { question: "Pick", options } });
    const group = screen.getByRole("group", { name: "Pick" });
    expect(group).toHaveClass("flex-wrap");
    expect(within(group).getAllByRole("button")).toHaveLength(6);
    expect(within(group).getByRole("button", { name: "Four" })).toHaveAttribute("aria-keyshortcuts", "4");
    expect(within(group).getByRole("button", { name: "Five" })).not.toHaveAttribute("aria-keyshortcuts");
    fireEvent.keyDown(document.body, { key: "5" });
    expect(backend.callsTo("voice_pick")).toEqual([]);
  });

  it("Esc closes and stops speech, unless a dialog or menu is open or it was handled", () => {
    render(<OpenDotStage />);
    show({ state: "speaking", said: ["Hello."] });

    const dialog = document.createElement("div");
    dialog.setAttribute("role", "dialog");
    document.body.append(dialog);
    fireEvent.keyDown(document.body, { key: "Escape" });
    dialog.setAttribute("role", "menu");
    fireEvent.keyDown(document.body, { key: "Escape" });
    dialog.remove();
    const handled = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    handled.preventDefault();
    act(() => void document.body.dispatchEvent(handled));
    expect(backend.callsTo("voice_close")).toEqual([]);
    expect(stage()).not.toBeNull();

    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(backend.callsTo("voice_close")).toEqual([{}]);
    // It closes at once, before the core says it went idle.
    expect(stage()).toBeNull();
  });

  it("Close closes too", () => {
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true });
    fireEvent.click(screen.getByRole("button", { name: /Close/ }));
    expect(backend.callsTo("voice_close")).toEqual([{}]);
    expect(stage()).toBeNull();
  });

  it("clicking The Breath stops speech and listens", () => {
    render(<OpenDotStage />);
    show({ state: "speaking", said: ["Hello."] });
    fireEvent.click(screen.getByRole("button", { name: "Stop and listen" }));
    expect(backend.callsTo("voice_stop")).toEqual([{}]);
  });

  it("shows a proposal card", () => {
    render(<OpenDotStage />);
    show({ state: "asking", card: { kind: "proposal", proposal } });
    const card = stage()!.querySelector<HTMLElement>("[data-card]")!;
    expect(within(card).getByText("Downloads tidy")).toBeVisible();
    expect(within(card).getByText(/Read the Downloads folder/)).toHaveClass("line-clamp-6");
    expect(within(card).getByText("Weekdays at 09:00")).toBeVisible();
    expect(within(card).getByText("C:\\Users\\me\\Downloads")).toBeVisible();
    expect(within(card).getByText("Start with this week's files.")).toBeVisible();
    // Long instructions can be read in full.
    show({
      card: { kind: "proposal", proposal: { ...proposal, instructions: "word ".repeat(200), schedule: null, folder: null } },
    });
    const all = within(card).getByRole("button", { name: "Show all" });
    fireEvent.click(all);
    expect(within(card).getByText(/word word/)).not.toHaveClass("line-clamp-6");
    expect(within(card).getByText("Only when you talk to it")).toBeVisible();
    expect(within(card).getByText("A new folder")).toBeVisible();
  });

  it.each<[OpenDotCard, string[]]>([
    [
      { kind: "confirm", deed: { kind: "approve", approval_id: "a", dot_name: "triage", tool: "Bash", target: "rm -rf ./build" } },
      ["Allow", "triage", "Bash", "rm -rf ./build"],
    ],
    [
      { kind: "confirm", deed: { kind: "deny", approval_id: "a", dot_name: "triage", tool: "Write", target: "notes.md" } },
      ["Deny", "triage", "Write", "notes.md"],
    ],
    [{ kind: "confirm", deed: { kind: "run", dot_id: "d", dot_name: "digest" } }, ["Run now", "digest"]],
    [
      { kind: "confirm", deed: { kind: "send", dot_id: "d", dot_name: "digest", text: "Add Friday.", topic_root: null } },
      ["Send", "digest", "Add Friday."],
    ],
    [
      { kind: "confirm", deed: { kind: "answer", message_id: "m", dot_name: "digest", text: "Yes, go ahead." } },
      ["Answer", "digest", "Yes, go ahead."],
    ],
    [{ kind: "list", title: "Waiting", lines: ["One approval", "Two notes"] }, ["Waiting", "One approval", "Two notes"]],
  ])("shows a %s card", (card, texts) => {
    render(<OpenDotStage />);
    show({ state: "asking", card });
    const area = stage()!.querySelector<HTMLElement>("[data-card]")!;
    for (const text of texts) expect(within(area).getByText(text)).toBeVisible();
    if (card.kind === "confirm" && (card.deed.kind === "approve" || card.deed.kind === "deny")) {
      expect(within(area).getByText(card.deed.target)).toHaveClass("font-mono", "wrap-anywhere");
    }
  });

  it("lingers 2.5 s after going idle, done then resting, then closes", () => {
    vi.useFakeTimers();
    render(<OpenDotStage />);
    show({ state: "speaking", said: ["All done."] });
    expect(breathState()).toBe("speaking");
    show({ state: "idle" });
    expect(stage()).not.toBeNull();
    expect(words()).toHaveTextContent("All done.");
    expect(breathState()).toBe("done");
    act(() => void vi.advanceTimersByTime(DONE_MS));
    expect(breathState()).toBe("resting");
    act(() => void vi.advanceTimersByTime(LINGER_MS - DONE_MS - 1));
    expect(stage()).not.toBeNull();
    act(() => void vi.advanceTimersByTime(1));
    expect(stage()).toBeNull();
    // Listening again opens it again.
    show({ state: "listening", micOpen: true });
    expect(stage()).not.toBeNull();
  });

  it("says why the microphone is not open", () => {
    render(<OpenDotStage />);
    const footer = () => stage()!.querySelector("[data-footer-line]");
    show({ state: "listening", micOpen: true });
    expect(footer()).toBeNull();
    show({ state: "thinking", micOpen: false });
    expect(footer()).toHaveTextContent("Thinking…");
    show({ state: "speaking", micOpen: false });
    expect(footer()).toHaveTextContent("Speaking. Click the dots or press the shortcut to interrupt.");
    show({ state: "idle", micOpen: false, ask: { question: "Run it?", options: ["Yes", "No"] } });
    expect(footer()).toHaveTextContent(
      "Not listening. Click the dots to answer by voice, or pick an option.",
    );
    show({ trouble: "No microphone was found." });
    expect(footer()).toHaveTextContent("No microphone was found.");
  });

  it("does not draw again for a level", () => {
    let renders = 0;
    render(
      <Profiler id="stage" onRender={() => (renders += 1)}>
        <OpenDotStage />
      </Profiler>,
    );
    show({ state: "speaking", said: ["Hello."], ask: { question: "Q", options: ["A", "B"] } });
    const before = renders;
    act(() => {
      for (let i = 0; i < 30; i++) voiceStore.set({ level: { mic: i / 30, voice: (30 - i) / 30 } });
    });
    expect(renders).toBe(before);
  });

  it("keeps long content inside the panel", () => {
    render(<OpenDotStage />);
    const long = "x".repeat(400);
    show({
      state: "asking",
      said: [long],
      ask: { question: "Q", options: [long, "B"] },
      card: { kind: "list", title: long, lines: Array.from({ length: 40 }, (_, i) => `${long} ${i}`) },
    });
    const panel = stage()!;
    expect(panel.className).toMatch(/max-h-/);
    const card = panel.querySelector<HTMLElement>("[data-card]")!;
    expect(card).toHaveClass("overflow-y-auto", "min-h-0", "wrap-anywhere");
    expect(card.className).toMatch(/max-h-/);
    expect(within(words()).getByText(long)).toHaveClass("line-clamp-2", "wrap-anywhere");
    // The options never give up their place to the card.
    expect(screen.getByRole("group", { name: "Q" })).toHaveClass("shrink-0");
    expect(within(screen.getByRole("group", { name: "Q" })).getAllByRole("button")[0]).toHaveClass(
      "wrap-anywhere",
    );
  });

  it("cleared takes away the options and the card", () => {
    render(<OpenDotStage />);
    show({
      state: "asking",
      ask: { question: "Q", options: ["A", "B"] },
      card: { kind: "list", title: "Waiting", lines: ["One"] },
    });
    expect(screen.getByRole("group", { name: "Q" })).toBeVisible();
    show({ ask: null, card: null, hearing: "", saying: "", said: [] });
    expect(screen.queryByRole("group", { name: "Q" })).toBeNull();
    expect(stage()?.querySelector("[data-card]") ?? null).toBeNull();
  });

  it("only fades with reduced motion", () => {
    reducedMotion(true);
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true });
    expect(stage()!.className).toMatch(/fade-in/);
    expect(stage()!.className).not.toMatch(/slide-in/);
  });

  it("rises and fades otherwise", () => {
    reducedMotion(false);
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true });
    expect(stage()!.className).toMatch(/fade-in/);
    expect(stage()!.className).toMatch(/slide-in-from-bottom/);
  });

  it("is reached by Tab in order: The Breath, the options, then Close", async () => {
    const user = userEvent.setup();
    render(
      <>
        <button type="button">Before</button>
        <OpenDotStage />
      </>,
    );
    show({ state: "asking", micOpen: true, ask: { question: "Q", options: ["A", "B"] } });
    screen.getByRole("button", { name: "Before" }).focus();
    const expected = [
      screen.getByRole("button", { name: "Stop and listen" }),
      screen.getByRole("button", { name: "A" }),
      screen.getByRole("button", { name: "B" }),
      screen.getByRole("button", { name: "Close" }),
    ];
    for (const element of expected) {
      await user.tab();
      expect(document.activeElement).toBe(element);
    }
  });
});
