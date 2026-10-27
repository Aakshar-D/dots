import { useEffect } from "react";

import { commands } from "@/api/commands";

/// How often keyboard and pointer input is told to the core at most.
const EVERY_MS = 30_000;

let composerHasText = false;
let lastSent = 0;

function send(): void {
  lastSent = Date.now();
  commands.reportActivity(composerHasText).catch(() => {});
}

/// The composer's text became empty or not: told at once, so an idle install never closes
/// Dots over a message being written (updates UI spec section 4.2).
export function reportComposer(hasText: boolean): void {
  if (hasText === composerHasText) return;
  composerHasText = hasText;
  send();
}

/// What counts as the person using the window: reading with the wheel or moving the pointer
/// too, not only typing and clicking.
const INPUT_EVENTS = ["keydown", "pointerdown", "pointermove", "wheel", "focus"] as const;

/// Tells the core about input in the window, at most every 30 s.
export function useActivity(): void {
  useEffect(() => {
    const onInput = () => {
      if (Date.now() - lastSent >= EVERY_MS) send();
    };
    for (const name of INPUT_EVENTS) window.addEventListener(name, onInput, { passive: true });
    return () => {
      for (const name of INPUT_EVENTS) window.removeEventListener(name, onInput);
    };
  }, []);
}

/// The composer's emptiness for the idle install, and "empty" once the composer goes away, so
/// text left in a closed conversation does not block it for the session.
export function useComposerReport(hasText: boolean): void {
  useEffect(() => reportComposer(hasText), [hasText]);
  useEffect(() => () => reportComposer(false), []);
}
