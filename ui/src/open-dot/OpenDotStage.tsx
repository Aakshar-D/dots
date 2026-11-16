import { cn } from "cn";
import { Copy, Maximize2, Minimize2 } from "lucide-react";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import { notifyError } from "@/lib/notify";
import { useVoiceLive, useVoiceStatus, voiceStore, type VoiceLive } from "@/lib/use-voice";

import { Breath } from "./Breath";
import { breathStateOf } from "./breath-model";
import { Card } from "./cards";
import { useVoiceHotkey } from "./RailMark";
import { interrupts, LEAVE_MS, talk, useStage, type StageCommands } from "./use-stage";

/// Options past this many show without a key hint; keys 1 to 4 pick.
const KEYED = 4;

const REDUCE = "(prefers-reduced-motion: reduce)";

/// Where the choice of the enlarged stage is kept on this computer.
const LARGE_KEY = "open-sage-large";

function readLarge(): boolean {
  try {
    return window.localStorage.getItem(LARGE_KEY) === "1";
  } catch {
    return false;
  }
}

function keepLarge(large: boolean): void {
  try {
    if (large) window.localStorage.setItem(LARGE_KEY, "1");
    else window.localStorage.removeItem(LARGE_KEY);
  } catch {
    // The choice then lasts until the app closes.
  }
}

function copy(text: string): void {
  navigator.clipboard
    .writeText(text)
    .then(() => toast.success("Copied"))
    .catch(notifyError);
}

/// The conversation as plain text, one paragraph per turn.
export function talkText(talk: VoiceLive["talk"]): string {
  return talk.map((line) => `${line.who === "you" ? "You" : "Sage"}: ${line.text}`).join("\n\n");
}

/// Whether the person asked for less motion, following the setting as it changes.
function useReducedMotion(): boolean {
  return useSyncExternalStore(
    (changed) => {
      const media = typeof window.matchMedia === "function" ? window.matchMedia(REDUCE) : null;
      media?.addEventListener?.("change", changed);
      return () => media?.removeEventListener?.("change", changed);
    },
    () => (typeof window.matchMedia === "function" ? window.matchMedia(REDUCE).matches : false),
  );
}

/// Whether a key press belongs to a field the person is typing in.
function typing(target: EventTarget | null): boolean {
  const el = target instanceof HTMLElement ? target : document.activeElement;
  if (!(el instanceof HTMLElement)) return false;
  return (
    el.isContentEditable ||
    el.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']") !== null
  );
}

/// Whether something else on screen owns the keys now: a dialog (the search box is one), a
/// menu or a list box.
function somethingElseIsOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]') !== null;
}

/// The one line under the words that says why the microphone is not open; null while it is.
function micLine(live: {
  state: VoiceLive["state"];
  micOpen: boolean;
  trouble: string | null;
  showing: boolean;
  options: boolean;
  shortcut: boolean;
}): string | null {
  if (live.micOpen) return null;
  if (live.trouble) return live.trouble;
  switch (live.state) {
    case "thinking":
      return "Thinking…";
    case "speaking":
      return live.shortcut
        ? "Speaking. Click the mark or press the shortcut to interrupt."
        : "Speaking. Click the mark to interrupt.";
    default:
      if (!live.showing) return null;
      return live.options
        ? "Not listening. Click the mark to answer by voice, or pick an option."
        : "Not listening. Click the mark to answer by voice.";
  }
}

/// What it hears while listening, then the reply: two lines at most, the newest strongest.
/// Only finished things are read out: the reply's lines and the sentence once heard. Words
/// still coming are shown but kept out of the live region. A newest reply line that only
/// repeats the question printed below it, whole or one sentence of it, is left out.
function Words({ question }: { question: string | null }) {
  const state = useVoiceLive((live) => live.state);
  const hearing = useVoiceLive((live) => live.hearing);
  const heard = useVoiceLive((live) => live.heard);
  const said = useVoiceLive((live) => live.said);
  // The question is spoken a sentence at a time, so its lines end the reply: they are dropped
  // from the end only, and a line before them that merely holds the same words stays.
  let kept = said.length;
  if (question !== null) {
    while (kept > 0) {
      const text = said[kept - 1]?.text.trim() ?? "";
      if (text === "" || !question.includes(text)) break;
      kept -= 1;
    }
  }
  const shown = said.slice(0, kept);
  const reply = state === "listening" ? [] : shown.slice(-2);
  const finished = reply.length === 0 && hearing !== "" && heard;
  const coming = reply.length === 0 && hearing !== "" && !heard;
  // The live region is always on the page and never hidden: one that is `display: none` when
  // a turn starts may not announce its first line. Empty, it takes no height; the words still
  // coming only show while it is empty, so no gap is kept between the two.
  return (
    <div className="flex w-full shrink-0 flex-col items-center text-center">
      <div aria-live="polite" className="flex w-full flex-col items-center gap-1">
        {reply.map((line, i) => {
          const last = i === reply.length - 1;
          return (
            <p
              key={line.n}
              title={line.text}
              className={cn(
                "max-w-full wrap-anywhere",
                last ? "line-clamp-2 text-sm text-foreground" : "line-clamp-1 text-xs text-muted-foreground",
              )}
            >
              {line.text}
            </p>
          );
        })}
        {finished && (
          <p title={hearing} className="line-clamp-2 max-w-full text-sm text-muted-foreground wrap-anywhere">
            {hearing}
          </p>
        )}
      </div>
      {coming && (
        <p
          aria-live="off"
          title={hearing}
          className="line-clamp-2 max-w-full text-sm text-muted-foreground wrap-anywhere"
        >
          {hearing}
        </p>
      )}
    </div>
  );
}

/// The enlarged stage's words: the whole conversation, every reply in full, in text that can
/// be selected and copied. The newest is kept in view. Words still coming show below it.
function Talk() {
  const talk = useVoiceLive((live) => live.talk);
  const hearing = useVoiceLive((live) => live.hearing);
  const heard = useVoiceLive((live) => live.heard);
  const end = useRef<HTMLDivElement>(null);
  const newest = talk[talk.length - 1];
  const tail = `${newest?.n ?? 0}:${newest?.text.length ?? 0}:${heard ? "" : hearing}`;
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: "end" });
  }, [tail]);
  const coming = hearing !== "" && !heard;
  return (
    <div
      data-talk=""
      role="log"
      aria-label="Conversation"
      tabIndex={0}
      className="flex min-h-0 w-full flex-1 flex-col gap-4 overflow-y-auto rounded-lg bg-muted/40 px-5 py-4 text-left select-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
    >
      {talk.length === 0 && !coming && (
        <p className="m-auto text-sm text-muted-foreground">What you say and what Sage answers shows here.</p>
      )}
      {talk.map((line) => (
        <div key={line.n} data-who={line.who} className="group flex flex-col gap-1">
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-muted-foreground">{line.who === "you" ? "You" : "Sage"}</span>
            {line.who === "sage" && (
              <button
                type="button"
                aria-label="Copy this reply"
                onClick={() => copy(line.text)}
                className="inline-flex items-center rounded p-1 text-muted-foreground opacity-0 group-hover:opacity-100 hover:bg-muted hover:text-foreground focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-ring"
              >
                <Copy aria-hidden className="size-3.5" />
              </button>
            )}
          </div>
          <p
            className={cn(
              "text-base leading-relaxed whitespace-pre-wrap wrap-anywhere",
              line.who === "you" ? "text-muted-foreground" : "text-foreground",
            )}
          >
            {line.text}
          </p>
        </div>
      ))}
      {coming && (
        <p aria-live="off" className="text-base leading-relaxed text-muted-foreground/70 wrap-anywhere">
          {hearing}
        </p>
      )}
      <div ref={end} />
    </div>
  );
}

/// The question, printed above its options; an open question says how to answer it.
function Question({ ask }: { ask: NonNullable<VoiceLive["ask"]> }) {
  return (
    <div className="flex w-full shrink-0 flex-col items-center gap-0.5 text-center">
      <p
        data-question=""
        title={ask.question}
        className="line-clamp-2 max-w-full text-sm font-medium text-foreground wrap-anywhere"
      >
        {ask.question}
      </p>
      {ask.options.length === 0 && <p className="text-xs text-muted-foreground">Say your answer.</p>}
    </div>
  );
}

/// The question's options as pills, numbered 1 to 4. A pick waits for the next question: a
/// second click or key while the first is on its way does nothing.
function Options(props: {
  ask: NonNullable<VoiceLive["ask"]>;
  picked: number | null;
  pick: (index: number) => void;
}) {
  const { ask, picked, pick } = props;
  return (
    <div role="group" aria-label={ask.question} className="flex w-full shrink-0 flex-wrap justify-center gap-2">
      {ask.options.map((option, i) => {
        const keyed = i < KEYED;
        return (
          <button
            key={i}
            type="button"
            aria-keyshortcuts={keyed ? String(i + 1) : undefined}
            aria-disabled={picked !== null || undefined}
            data-picked={picked === i || undefined}
            onClick={() => pick(i)}
            className={cn(
              "inline-flex max-w-full items-center gap-2 rounded-full border px-3 py-1.5 text-left text-sm wrap-anywhere transition-colors",
              "hover:bg-muted focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
              picked === i && "border-primary bg-primary/10 text-foreground",
              picked !== null && picked !== i && "opacity-50 hover:bg-transparent",
            )}
          >
            {keyed && (
              <kbd aria-hidden className="font-sans text-xs text-muted-foreground tabular-nums">
                {i + 1}
              </kbd>
            )}
            <span className="min-w-0">{option}</span>
          </button>
        );
      })}
    </div>
  );
}

/// The stage: what a person sees while talking to Dot. It floats above whatever screen shows,
/// at the bottom in the middle, with The Breath, the live words, the question, its options and
/// a card. Once closed it fades away for `LEAVE_MS`, out of reach of the keyboard, the mouse
/// and a screen reader, and then renders nothing, so The Breath inside it stops with it.
export function OpenDotStage({ act = commands }: { act?: StageCommands } = {}) {
  // Enlarged, the stage fills the window and shows the whole conversation to read and copy.
  const [large, setLarge] = useState(readLarge);
  const resize = () => {
    keepLarge(!large);
    setLarge(!large);
  };
  const stage = useStage(act, large);
  // Whether the panel is drawn: while open, and for `LEAVE_MS` after. Adjusted while
  // rendering, so it never misses a frame between open and leaving.
  const [drawn, setDrawn] = useState(stage.open);
  if (stage.open && !drawn) setDrawn(true);
  const leaving = drawn && !stage.open;
  useEffect(() => {
    if (!leaving) return;
    const gone = setTimeout(() => {
      setDrawn(false);
      // The conversation is shown for as long as the stage is: gone with it.
      voiceStore.set({ talk: [] });
    }, LEAVE_MS);
    return () => clearTimeout(gone);
  }, [leaving]);
  if (!drawn) return null;
  return (
    <Panel
      act={act}
      close={stage.close}
      justFinished={stage.justFinished}
      leaving={leaving}
      large={large}
      resize={resize}
    />
  );
}

function Panel(props: {
  act: StageCommands;
  close: () => void;
  justFinished: boolean;
  leaving: boolean;
  large: boolean;
  resize: () => void;
}) {
  const { act, close, justFinished, leaving, large, resize } = props;
  const spoken = useVoiceLive((live) => live.talk.length > 0);
  const state = useVoiceLive((live) => live.state);
  const micOpen = useVoiceLive((live) => live.micOpen);
  const trouble = useVoiceLive((live) => live.trouble);
  const ask = useVoiceLive((live) => live.ask);
  const card = useVoiceLive((live) => live.card);
  const status = useVoiceStatus().data;
  const hotkey = useVoiceHotkey();
  const reduced = useReducedMotion();

  // The question a pick was sent for, and which option: none until the next question, or
  // until the pick fails, so the options can be tried again.
  const [pickedFor, setPickedFor] = useState<{ ask: VoiceLive["ask"]; index: number } | null>(null);
  const picked = ask !== null && pickedFor?.ask === ask ? pickedFor.index : null;
  const pick = (index: number) => {
    if (ask === null || picked !== null || index < 0 || index >= ask.options.length) return;
    const sent = { ask, index };
    setPickedFor(sent);
    // The seq of the question these options were rendered from, not the store's latest: a
    // pick made as a newer question arrives names the old one, and the app ignores it.
    act.voicePick(index, ask.seq).catch((e: unknown) => {
      setPickedFor((p) => (p === sent ? null : p));
      notifyError(e);
    });
  };

  // One pair of key listeners while the stage shows; they read the latest of what they need.
  // Whether something else owns the keys is decided as the key goes down, in the capture
  // phase before any handler runs: a menu that closes on this very `Esc` is gone by the time
  // the key bubbles up to the window.
  const latest = useRef({ close, pick, ask, leaving });
  latest.current = { close, pick, ask, leaving };
  useEffect(() => {
    let decided: { event: KeyboardEvent; elsewhere: boolean } | null = null;
    const onCapture = (e: KeyboardEvent) => {
      decided = { event: e, elsewhere: somethingElseIsOpen() };
    };
    const onKey = (e: KeyboardEvent) => {
      const elsewhere = decided?.event === e ? decided.elsewhere : somethingElseIsOpen();
      decided = null;
      const now = latest.current;
      if (now.leaving || elsewhere) return;
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
      // On purpose before the typing check: `Esc` closes the stage even when focus is in a
      // text field (the search box or a composer). Nothing on the stage is typed into, and a
      // field's own `Esc` (a menu, a dialog) is caught above.
      if (e.key === "Escape") {
        e.preventDefault();
        now.close();
        return;
      }
      // The digit keys, and the number pad's (its key is the digit while Num Lock is on).
      if (e.repeat || !/^[1-4]$/.test(e.key) || now.ask === null || typing(e.target)) return;
      const index = Number(e.key) - 1;
      if (index >= now.ask.options.length) return;
      e.preventDefault();
      now.pick(index);
    };
    window.addEventListener("keydown", onCapture, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onCapture, true);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  // Until the voice status is known, or while voice cannot listen, the dots do nothing.
  const ready = status?.enabled === true && status.ready;
  const breath = breathStateOf({ state, trouble, ask }, justFinished);
  const options = ask !== null && ask.options.length > 0;
  const line = micLine({
    state,
    micOpen,
    trouble,
    showing: ask !== null || card !== null,
    options,
    shortcut: hotkey !== "",
  });

  return (
    <div
      aria-hidden={leaving || undefined}
      className={cn(
        "pointer-events-none fixed inset-x-0 bottom-0 z-40 flex justify-center pr-4 pb-4 pl-[calc(46px+1rem)]",
        large && "top-0 pt-4",
      )}
    >
      <section
        role="region"
        aria-label="Open Sage"
        data-large={large || undefined}
        inert={leaving}
        className={cn(
          "flex max-h-[calc(100dvh-2rem)] w-full flex-col items-center gap-3 rounded-xl bg-popover px-6 pt-5 pb-3 text-popover-foreground shadow-lg ring-1 ring-foreground/10",
          large ? "h-full max-w-5xl" : "max-w-xl",
          leaving
            ? "pointer-events-none animate-out duration-200 ease-in fill-mode-forwards fade-out-0"
            : "pointer-events-auto animate-in duration-250 ease-out fade-in-0",
          !reduced && (leaving ? "slide-out-to-bottom-4" : "slide-in-from-bottom-4"),
        )}
      >
        <div data-breath-state={breath} className={cn("shrink-0", large ? "p-1" : "p-3")}>
          <Breath
            size={large ? 84 : 132}
            state={breath}
            options={ask?.options.length ?? 3}
            label={interrupts(state) ? "Stop and listen" : "Listen"}
            onClick={() => {
              if (ready) talk(act, state).catch(notifyError);
            }}
          />
        </div>
        {large ? <Talk /> : <Words question={ask?.question ?? null} />}
        {ask !== null && <Question ask={ask} />}
        {options && <Options ask={ask} picked={picked} pick={pick} />}
        {card !== null && (
          <div
            data-card=""
            role="group"
            aria-label="Details"
            tabIndex={0}
            className={cn(
              "min-h-0 w-full shrink overflow-y-auto rounded-lg bg-muted/60 px-3 py-2.5 leading-relaxed wrap-anywhere select-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
              large ? "max-h-[40%] shrink-0 text-sm" : "max-h-48 text-xs",
            )}
          >
            <Card card={card} />
          </div>
        )}
        <div className="flex w-full shrink-0 items-center gap-3 border-t pt-2.5">
          {line !== null ? (
            <p
              data-footer-line=""
              className={cn(
                "line-clamp-2 min-w-0 flex-1 text-xs",
                trouble && !micOpen ? "text-destructive" : "text-muted-foreground",
              )}
            >
              {line}
            </p>
          ) : (
            <span className="flex-1" />
          )}
          {large && spoken && (
            <button
              type="button"
              onClick={() => copy(talkText(voiceStore.get().talk))}
              className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            >
              <Copy aria-hidden className="size-3.5" />
              Copy all
            </button>
          )}
          <button
            type="button"
            aria-pressed={large}
            aria-label={large ? "Make Open Sage smaller" : "Enlarge Open Sage"}
            title={large ? "Smaller" : "Enlarge to read and copy"}
            onClick={resize}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          >
            {large ? <Minimize2 aria-hidden className="size-3.5" /> : <Maximize2 aria-hidden className="size-3.5" />}
            {large ? "Smaller" : "Enlarge"}
          </button>
          <button
            type="button"
            aria-keyshortcuts="Escape"
            onClick={close}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          >
            Close
            <kbd aria-hidden className="rounded border px-1 font-sans text-[10px] leading-4">
              Esc
            </kbd>
          </button>
        </div>
      </section>
    </div>
  );
}
