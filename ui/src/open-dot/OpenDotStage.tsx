import { cn } from "cn";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import { commands } from "@/api/commands";
import { notifyError } from "@/lib/notify";
import { useVoiceLive, type VoiceLive } from "@/lib/use-voice";

import { Breath } from "./Breath";
import { breathStateOf } from "./breath-model";
import { Card } from "./cards";
import { useStage, type StageCommands } from "./use-stage";

/// Options past this many show without a key hint; keys 1 to 4 pick.
const KEYED = 4;

const REDUCE = "(prefers-reduced-motion: reduce)";

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

/// Whether something else on screen owns `Esc` now: a dialog, a menu or a list box.
function somethingElseIsOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"]') !== null;
}

/// The one line under the words that says why the microphone is not open; null while it is.
function micLine(live: {
  state: VoiceLive["state"];
  micOpen: boolean;
  trouble: string | null;
  showing: boolean;
}): string | null {
  if (live.micOpen) return null;
  if (live.trouble) return live.trouble;
  switch (live.state) {
    case "thinking":
      return "Thinking…";
    case "speaking":
      return "Speaking. Click the dots or press the shortcut to interrupt.";
    default:
      return live.showing ? "Not listening. Click the dots to answer by voice, or pick an option." : null;
  }
}

/// What it hears while listening, then the reply: two lines at most, the newest strongest.
function Words() {
  const state = useVoiceLive((live) => live.state);
  const hearing = useVoiceLive((live) => live.hearing);
  const said = useVoiceLive((live) => live.said);
  const reply = state === "listening" ? [] : said.slice(-2);
  return (
    <div aria-live="polite" className="flex w-full shrink-0 flex-col items-center gap-1 text-center">
      {reply.length > 0
        ? reply.map((line, i) => {
            const newest = i === reply.length - 1;
            return (
              <p
                key={`${said.length - reply.length + i}`}
                title={line}
                className={cn(
                  "max-w-full wrap-anywhere",
                  newest ? "line-clamp-2 text-sm text-foreground" : "line-clamp-1 text-xs text-muted-foreground",
                )}
              >
                {line}
              </p>
            );
          })
        : hearing !== "" && (
            <p title={hearing} className="line-clamp-2 max-w-full text-sm text-muted-foreground wrap-anywhere">
              {hearing}
            </p>
          )}
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
/// at the bottom in the middle, with The Breath, the live words, the options and a card. It
/// renders nothing while closed, so The Breath inside it stops with it.
export function OpenDotStage({ act = commands }: { act?: StageCommands } = {}) {
  const stage = useStage(act);
  if (!stage.open) return null;
  return <Panel act={act} close={stage.close} justFinished={stage.justFinished} />;
}

function Panel(props: { act: StageCommands; close: () => void; justFinished: boolean }) {
  const { act, close, justFinished } = props;
  const state = useVoiceLive((live) => live.state);
  const micOpen = useVoiceLive((live) => live.micOpen);
  const trouble = useVoiceLive((live) => live.trouble);
  const ask = useVoiceLive((live) => live.ask);
  const card = useVoiceLive((live) => live.card);
  const reduced = useReducedMotion();

  // The question a pick was sent for, and which option: none until the next question.
  const [pickedFor, setPickedFor] = useState<{ ask: VoiceLive["ask"]; index: number } | null>(null);
  const picked = ask !== null && pickedFor?.ask === ask ? pickedFor.index : null;
  const pick = (index: number) => {
    if (ask === null || picked !== null || index < 0 || index >= ask.options.length) return;
    setPickedFor({ ask, index });
    act.voicePick(index).catch(notifyError);
  };

  // One key listener while the stage shows; it reads the latest of what it needs.
  const latest = useRef({ close, pick, ask });
  latest.current = { close, pick, ask };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
      const now = latest.current;
      if (e.key === "Escape") {
        if (somethingElseIsOpen()) return;
        e.preventDefault();
        now.close();
        return;
      }
      if (!/^[1-4]$/.test(e.key) || now.ask === null || typing(e.target)) return;
      const index = Number(e.key) - 1;
      if (index >= now.ask.options.length) return;
      e.preventDefault();
      now.pick(index);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const breath = breathStateOf({ state, trouble, ask }, justFinished);
  const line = micLine({ state, micOpen, trouble, showing: ask !== null || card !== null });

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-0 z-40 flex justify-center pr-4 pb-4 pl-[calc(46px+1rem)]">
      <section
        role="region"
        aria-label="Open Dot"
        className={cn(
          "pointer-events-auto flex max-h-[calc(100dvh-2rem)] w-full max-w-xl flex-col items-center gap-3 rounded-xl bg-popover px-6 pt-5 pb-3 text-popover-foreground shadow-lg ring-1 ring-foreground/10",
          "animate-in duration-250 ease-out fade-in-0",
          !reduced && "slide-in-from-bottom-4",
        )}
      >
        <div data-breath-state={breath} className="shrink-0 p-3">
          <Breath
            size={132}
            state={breath}
            options={ask?.options.length ?? 3}
            onClick={() => act.voiceStop().catch(notifyError)}
          />
        </div>
        <Words />
        {ask !== null && ask.options.length > 0 && <Options ask={ask} picked={picked} pick={pick} />}
        {card !== null && (
          <div
            data-card=""
            role="group"
            aria-label="Details"
            tabIndex={0}
            className="max-h-48 min-h-0 w-full shrink overflow-y-auto rounded-lg bg-muted/60 px-3 py-2.5 text-xs leading-relaxed wrap-anywhere focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
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
