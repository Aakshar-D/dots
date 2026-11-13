import { QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render as rtlRender, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Profiler, type ReactNode } from "react";
import { toast } from "sonner";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OpenDotCard, SetupProposal } from "@/api/types";
import { createQueryClient, keys } from "@/lib/query";
import { voiceStore, type VoiceLive } from "@/lib/use-voice";
import { backend } from "@/test/backend";
import { sample } from "@/test/fixtures";

import { DONE_MS } from "./breath-model";
import { OpenDotStage } from "./OpenDotStage";
import { LEAVE_MS, LINGER_MS, TROUBLE_MS, type StageCommands } from "./use-stage";

/// The stage reads the voice status and the shortcut from the query cache, as in the app.
function render(ui: ReactNode, hotkey = "CmdOrCtrl+Shift+Space") {
  const qc = createQueryClient();
  qc.setQueryData(keys.voice, { ...sample.voiceReady, trouble: null, mic_open: false, state: "idle" });
  qc.setQueryData(keys.settings, { ...sample.settings, voice_hotkey: hotkey });
  return rtlRender(ui, {
    wrapper: ({ children }) => <QueryClientProvider client={qc}>{children}</QueryClientProvider>,
  });
}

/// Reply lines with running numbers, as the store keeps them.
let line = 0;
const lines = (...texts: string[]) => texts.map((text) => ({ n: ++line, text }));

const show = (change: Partial<VoiceLive>) => act(() => voiceStore.set(change));
const stage = () => screen.queryByRole("region", { name: "Open Sage" });
const words = () => stage()!.querySelector<HTMLElement>("[aria-live='polite']")!;
const question = () => stage()!.querySelector<HTMLElement>("[data-question]");
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

describe("the Open Sage stage", () => {
  it("does not render at all while closed", () => {
    show({ state: "idle" });
    const { container } = render(<OpenDotStage />);
    expect(stage()).toBeNull();
    expect(container.querySelector("svg")).toBeNull();
  });

  it("opens on listening and shows what it hears, announcing it only once heard", () => {
    show({ state: "idle" });
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true, hearing: "what is waiting" });
    expect(stage()).toBeVisible();
    const hearing = within(stage()!).getByText("what is waiting");
    expect(hearing).toHaveClass("text-muted-foreground");
    // Words still coming are shown but not read out.
    expect(hearing.closest("[aria-live]")).toHaveAttribute("aria-live", "off");
    expect(words()).not.toHaveTextContent("what is waiting");
    // The finished sentence is.
    show({ hearing: "what is waiting for me", heard: true });
    expect(words()).toHaveTextContent("what is waiting for me");
    // Focus stays where it was.
    expect(document.activeElement).toBe(document.body);
  });

  it("keeps the spoken lines' live region in place while empty, so its first line is read out", () => {
    render(<OpenDotStage />);
    show({ state: "listening", micOpen: true, hearing: "" });
    const region = words();
    expect(region).toBeEmptyDOMElement();
    // Not taken out of the page while empty: a region that is `display: none` when a turn
    // starts may not announce the first thing put in it.
    expect(region.className).not.toMatch(/(^|\s|:)(hidden|invisible|sr-only)(\s|$)/);
    expect(region.style.display).not.toBe("none");
    expect(region.style.visibility).not.toBe("hidden");
    expect(region).toBeVisible();
    // The same node then carries the reply.
    show({ state: "speaking", said: lines("Here it is.") });
    expect(words()).toBe(region);
    expect(region).toHaveTextContent("Here it is.");
  });

  it("keeps each reply line's node, so a held line is not read out again", () => {
    render(<OpenDotStage />);
    const [one, two, three, four] = lines("One.", "Two.", "Three.", "Four.");
    show({ state: "speaking", said: [one!, two!, three!] });
    const held = within(words()).getByText("Three.");
    // The store holds three lines: a fourth drops the oldest, and the line still shown keeps
    // its node (a reused node with new text would be read out as new).
    show({ said: [two!, three!, four!] });
    expect(within(words()).getByText("Three.")).toBe(held);
    expect(held).toHaveTextContent(/^Three\.$/);
  });

  it("shows the reply's two newest lines, the newest strongest", () => {
    render(<OpenDotStage />);
    show({
      state: "speaking",
      micOpen: false,
      hearing: "",
      said: lines("First.", "Second.", "Third and newest."),
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
    show({ state: "asking", micOpen: true, ask: { question: "Run it now?", options: ["Yes", "Not now"], seq: 1 } });
    const group = screen.getByRole("group", { name: "Run it now?" });
    const options = within(group).getAllByRole("button");
    expect(options.map((o) => o.textContent)).toEqual(["1Yes", "2Not now"]);
    fireEvent.click(within(group).getByRole("button", { name: "Not now" }));
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1, seq: 1 }]);
    // A second click while that pick is on its way does nothing.
    fireEvent.click(within(group).getByRole("button", { name: "Yes" }));
    fireEvent.keyDown(document.body, { key: "1" });
    expect(backend.callsTo("voice_pick")).toHaveLength(1);
    // The next question can be answered again, and the pick names it.
    show({ ask: { question: "Which one?", options: ["A", "B"], seq: 2 } });
    fireEvent.click(screen.getByRole("button", { name: "A" }));
    expect(backend.callsTo("voice_pick")).toEqual([
      { index: 1, seq: 1 },
      { index: 0, seq: 2 },
    ]);
  });

  it("a pick sends the seq of the question its options were rendered from", () => {
    render(<OpenDotStage />);
    show({ state: "asking", micOpen: true, ask: { question: "Which?", options: ["A", "B"], seq: 41 } });
    fireEvent.keyDown(document.body, { key: "2" });
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1, seq: 41 }]);
  });

  it("a pick from a render made before a newer question sends the older seq", () => {
    render(<OpenDotStage />);
    show({ state: "asking", micOpen: true, ask: { question: "Approve it?", options: ["Approve", "Deny"], seq: 7 } });
    // The handler React gave the "Approve" button in this render: a click that lands as the
    // newer question arrives runs it.
    const button = screen.getByRole("button", { name: "Approve" });
    const propsKey = Object.keys(button).find((k) => k.startsWith("__reactProps$"))!;
    const staleClick = (button as unknown as Record<string, { onClick: () => void }>)[propsKey]!.onClick;
    show({ ask: { question: "Which sol?", options: ["Mail", "News"], seq: 8 } });
    act(() => staleClick());
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 0, seq: 7 }]);
  });

  it("number keys pick, unless typing or beyond the options", () => {
    render(
      <>
        <input aria-label="Notes" />
        <OpenDotStage />
      </>,
    );
    show({ state: "asking", micOpen: true, ask: { question: "Which?", options: ["A", "B"], seq: 1 } });
    const input = screen.getByRole("textbox", { name: "Notes" });
    input.focus();
    fireEvent.keyDown(input, { key: "2" });
    expect(backend.callsTo("voice_pick")).toEqual([]);
    input.blur();
    fireEvent.keyDown(document.body, { key: "3" });
    expect(backend.callsTo("voice_pick")).toEqual([]);
    fireEvent.keyDown(document.body, { key: "2" });
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1, seq: 1 }]);
  });

  it.each(["dialog", "alertdialog", "menu", "listbox"])(
    "number keys do not pick while a %s is open",
    (role) => {
      render(<OpenDotStage />);
      show({ state: "asking", micOpen: true, ask: { question: "Which?", options: ["A", "B"], seq: 1 } });
      const other = document.createElement("div");
      other.setAttribute("role", role);
      document.body.append(other);
      fireEvent.keyDown(document.body, { key: "1" });
      other.remove();
      expect(backend.callsTo("voice_pick")).toEqual([]);
    },
  );

  it("number keys ignore modifiers, composition and repeats, and take the number pad", () => {
    render(<OpenDotStage />);
    show({ state: "asking", micOpen: true, ask: { question: "Which?", options: ["A", "B"], seq: 1 } });
    for (const held of [{ ctrlKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }, { repeat: true }]) {
      fireEvent.keyDown(document.body, { key: "1", ...held });
    }
    expect(backend.callsTo("voice_pick")).toEqual([]);
    fireEvent.keyDown(document.body, { key: "2", code: "Numpad2" });
    expect(backend.callsTo("voice_pick")).toEqual([{ index: 1, seq: 1 }]);
  });

  it("Esc that closes a menu leaves the stage open, though the menu is gone by then", () => {
    render(<OpenDotStage />);
    show({ state: "speaking", said: lines("Hello.") });
    // A menu that closes on Esc from `document`, without saying it handled the key: by the time
    // the key reaches `window`, the menu is gone.
    const menu = document.createElement("div");
    menu.setAttribute("role", "menu");
    document.body.append(menu);
    const closeMenu = (e: KeyboardEvent) => {
      if (e.key === "Escape") menu.remove();
    };
    document.addEventListener("keydown", closeMenu);
    fireEvent.keyDown(document.body, { key: "Escape" });
    document.removeEventListener("keydown", closeMenu);
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(backend.callsTo("voice_close")).toEqual([]);
    expect(stage()).not.toBeNull();
    // The next Esc, with nothing else open, closes the stage.
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(backend.callsTo("voice_close")).toEqual([{}]);
  });

  it("shows key hints for the first four of more options, and wraps the rest", () => {
    render(<OpenDotStage />);
    const options = ["One", "Two", "Three", "Four", "Five", "Six"];
    show({ state: "asking", micOpen: true, ask: { question: "Pick", options, seq: 1 } });
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
    show({ state: "speaking", said: lines("Hello.") });

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

  it.each<[string, Partial<VoiceLive>, string, string]>([
    ["thinking", { state: "thinking" }, "Stop and listen", "voice_stop"],
    ["speaking", { state: "speaking" }, "Stop and listen", "voice_stop"],
    ["listening", { state: "listening", micOpen: true }, "Listen", "voice_open"],
    [
      "idle with a question waiting",
      { state: "idle", ask: { question: "Run it?", options: ["Yes", "No"], seq: 1 } },
      "Listen",
      "voice_open",
    ],
  ])("clicking The Breath while %s", (_, live, name, command) => {
    render(<OpenDotStage />);
    show({ said: lines("Hello."), ...live });
    fireEvent.click(screen.getByRole("button", { name }));
    expect(backend.callsTo(command)).toEqual([{}]);
    const other = command === "voice_stop" ? "voice_open" : "voice_stop";
    expect(backend.callsTo(other)).toEqual([]);
  });

  it("prints the question above the options, once", () => {
    render(<OpenDotStage />);
    show({ state: "asking", micOpen: true, said: lines("Triage wants to push."), ask: { question: "Allow it?", options: ["Allow", "Deny"], seq: 1 } });
    expect(question()).toHaveTextContent("Allow it?");
    expect(question()).toHaveClass("text-foreground", "font-medium", "line-clamp-2");
    // Above the options.
    const group = screen.getByRole("group", { name: "Allow it?" });
    expect(question()!.compareDocumentPosition(group) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Spoken as the newest reply line too: shown once.
    show({ said: lines("Triage wants to push.", "Allow it?") });
    expect(within(stage()!).getAllByText("Allow it?")).toHaveLength(1);
    expect(within(words()).getByText("Triage wants to push.")).toBeVisible();
    // A question spoken a sentence at a time: none of its sentences is printed twice, and
    // what was said before them stays.
    const long = "Downloads tidy. Runs every day at 08:00. Create this sol?";
    show({
      said: lines("Let me set that up.", "Downloads tidy.", "Runs every day at 08:00.", "Create this sol?"),
      ask: { question: long, options: ["Yes", "No"], seq: 2 },
    });
    expect(question()).toHaveTextContent(long);
    expect(within(words()).queryByText("Create this sol?")).toBeNull();
    expect(within(words()).queryByText("Runs every day at 08:00.")).toBeNull();
    expect(within(words()).getByText("Let me set that up.")).toBeVisible();
  });

  it("asks an open question with no options", () => {
    render(<OpenDotStage />);
    show({ state: "idle", micOpen: false, ask: { question: "What should it be called?", options: [], seq: 1 } });
    expect(question()).toHaveTextContent("What should it be called?");
    expect(within(stage()!).getByText("Say your answer.")).toBeVisible();
    expect(screen.queryByRole("group", { name: "What should it be called?" })).toBeNull();
    const footer = stage()!.querySelector("[data-footer-line]");
    expect(footer).toHaveTextContent("Not listening. Click the sols to answer by voice.");
    expect(footer).not.toHaveTextContent(/pick an option/);
  });

  it("names the shortcut to interrupt only when one is set", () => {
    render(<OpenDotStage />, "");
    show({ state: "speaking", micOpen: false });
    expect(stage()!.querySelector("[data-footer-line]")).toHaveTextContent(/^Speaking\. Click the sols to interrupt\.$/);
  });

  it("lets the options be tried again when a pick fails", async () => {
    const failed = vi.spyOn(toast, "error").mockImplementation(() => "");
    const act_: StageCommands = {
      voiceOpen: async () => null,
      voiceClose: async () => null,
      voiceStop: async () => null,
      voicePick: () => Promise.reject(new Error("Sage is not asking anything now.")),
    };
    render(<OpenDotStage act={act_} />);
    show({ state: "asking", micOpen: true, ask: { question: "Q", options: ["A", "B"], seq: 1 } });
    await act(async () => void fireEvent.click(screen.getByRole("button", { name: "A" })));
    expect(failed).toHaveBeenCalledWith("Sage is not asking anything now.");
    expect(screen.getByRole("button", { name: "A" })).not.toHaveAttribute("aria-disabled");
    expect(screen.getByRole("button", { name: "A" })).not.toHaveAttribute("data-picked");
    failed.mockRestore();
  });

  it("fades out when it closes, out of reach, and is gone after", () => {
    vi.useFakeTimers();
    reducedMotion(false);
    const { container } = render(<OpenDotStage />);
    show({ state: "listening", micOpen: true });
    fireEvent.click(screen.getByRole("button", { name: /Close/ }));
    // Still drawn while it leaves, but hidden from the keyboard, the mouse and the reader.
    const leaving = container.querySelector<HTMLElement>("section")!;
    expect(leaving).not.toBeNull();
    expect(leaving.className).toMatch(/fade-out/);
    expect(leaving.className).toMatch(/slide-out-to-bottom/);
    expect(leaving).toHaveAttribute("inert");
    expect(leaving.className).toMatch(/pointer-events-none/);
    expect(stage()).toBeNull();
    act(() => void vi.advanceTimersByTime(LEAVE_MS));
    expect(container.querySelector("section")).toBeNull();
    expect(container.querySelector("svg")).toBeNull();
  });

  it("only fades out with reduced motion", () => {
    reducedMotion(true);
    const { container } = render(<OpenDotStage />);
    show({ state: "listening", micOpen: true });
    fireEvent.click(screen.getByRole("button", { name: /Close/ }));
    const leaving = container.querySelector<HTMLElement>("section")!;
    expect(leaving.className).toMatch(/fade-out/);
    expect(leaving.className).not.toMatch(/slide-out/);
  });

  it("comes back if it opens again while leaving", () => {
    vi.useFakeTimers();
    render(<OpenDotStage />);
    show({ state: "speaking" });
    fireEvent.click(screen.getByRole("button", { name: /Close/ }));
    expect(stage()).toBeNull();
    show({ state: "listening", micOpen: true });
    expect(stage()).not.toBeNull();
    act(() => void vi.advanceTimersByTime(LEAVE_MS * 2));
    expect(stage()).not.toBeNull();
    expect(stage()).not.toHaveAttribute("inert");
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
      ["Approve the request from triage: Bash", "rm -rf ./build"],
    ],
    [
      { kind: "confirm", deed: { kind: "deny", approval_id: "a", dot_name: "triage", tool: "Write", target: "notes.md" } },
      ["Deny the request from triage: Write", "notes.md"],
    ],
    // No possessive: a name ending in "s" reads the same way.
    [
      { kind: "confirm", deed: { kind: "approve", approval_id: "a", dot_name: "news", tool: "Write", target: "today.md" } },
      ["Approve the request from news: Write", "today.md"],
    ],
    [{ kind: "confirm", deed: { kind: "run", dot_id: "d", dot_name: "backup" } }, ["Run backup now"]],
    [
      { kind: "confirm", deed: { kind: "send", dot_id: "d", dot_name: "research", text: "Add Friday.", topic_root: null } },
      ["Send to research", "Add Friday."],
    ],
    [
      { kind: "confirm", deed: { kind: "answer", message_id: "m", dot_name: "research", text: "Yes, go ahead." } },
      ["Answer research", "Yes, go ahead."],
    ],
    [{ kind: "list", title: "Waiting", lines: ["One approval", "Two notes"] }, ["Waiting", "One approval", "Two notes"]],
  ])("shows a %s card", (card, texts) => {
    render(<OpenDotStage />);
    show({ state: "asking", card });
    const area = stage()!.querySelector<HTMLElement>("[data-card]")!;
    for (const text of texts) expect(within(area).getByText(text)).toBeVisible();
    // The deed is the heading, in the foreground's weight.
    expect(within(area).getByText(texts[0]!)).toHaveClass("font-medium", "text-foreground");
    if (card.kind === "confirm" && (card.deed.kind === "approve" || card.deed.kind === "deny")) {
      expect(within(area).getByText(card.deed.target)).toHaveClass("font-mono", "wrap-anywhere");
    }
  });

  it("shows an approval with no target as its sol and tool only", () => {
    render(<OpenDotStage />);
    show({
      state: "asking",
      card: { kind: "confirm", deed: { kind: "approve", approval_id: "a", dot_name: "triage", tool: "WebSearch", target: "" } },
    });
    const area = stage()!.querySelector<HTMLElement>("[data-card]")!;
    expect(within(area).getByText("Approve the request from triage: WebSearch")).toBeVisible();
    expect(area.querySelector("code")).toBeNull();
  });

  it("lingers 2.5 s after going idle, done then resting, then closes", () => {
    vi.useFakeTimers();
    render(<OpenDotStage />);
    show({ state: "speaking", said: lines("All done.") });
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

  it("opens on a trouble while resting, shows it, and closes after a while", () => {
    vi.useFakeTimers();
    render(<OpenDotStage />);
    expect(stage()).toBeNull();
    // The hotkey with no microphone: Dot stays idle, and only a trouble comes.
    show({ trouble: "The microphone could not be opened: no microphone was found", troubles: 1 });
    expect(stage()).not.toBeNull();
    expect(stage()).toHaveTextContent("The microphone could not be opened");
    act(() => void vi.advanceTimersByTime(TROUBLE_MS - 1));
    expect(stage()).not.toBeNull();
    act(() => void vi.advanceTimersByTime(1));
    expect(stage()).toBeNull();
    // A stage closed by the person opens for the next trouble too.
    show({ trouble: "I can't hear anything from the microphone.", troubles: 2 });
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
    expect(footer()).toHaveTextContent("Speaking. Click the sols or press the shortcut to interrupt.");
    show({ state: "idle", micOpen: false, ask: { question: "Run it?", options: ["Yes", "No"], seq: 1 } });
    expect(footer()).toHaveTextContent(
      "Not listening. Click the sols to answer by voice, or pick an option.",
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
    show({ state: "speaking", said: lines("Hello."), ask: { question: "Q", options: ["A", "B"], seq: 1 } });
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
      said: lines(long),
      ask: { question: "Q", options: [long, "B"], seq: 1 },
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
      ask: { question: "Q", options: ["A", "B"], seq: 1 },
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
    show({ state: "asking", micOpen: true, ask: { question: "Q", options: ["A", "B"], seq: 1 } });
    screen.getByRole("button", { name: "Before" }).focus();
    const expected = [
      screen.getByRole("button", { name: "Listen" }),
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
