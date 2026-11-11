import { useCallback, useEffect, useState } from "react";

import { commands } from "@/api/commands";
import type { VoiceState } from "@/api/types";
import { notifyError } from "@/lib/notify";
import { useVoiceLive } from "@/lib/use-voice";

import { DONE_MS } from "./breath-model";

/// How long the stage stays after Open Dot goes idle with nothing showing, so the last words
/// can be read.
export const LINGER_MS = 2500;

const BUSY: ReadonlySet<VoiceState> = new Set(["listening", "thinking", "speaking", "asking"]);

/// The commands the stage and the rail mark call; the demo page passes its own.
export type StageCommands = Pick<
  typeof commands,
  "voiceOpen" | "voiceClose" | "voiceStop" | "voicePick"
>;

/// Whether Open Dot is doing something the stage shows: busy, or a question or a card showing.
export function useOpenDotBusy(): boolean {
  const state = useVoiceLive((live) => live.state);
  const showing = useVoiceLive((live) => live.ask !== null || live.card !== null);
  return BUSY.has(state) || showing;
}

/// Whether the stage shows, and how it closes. It shows while Open Dot is busy or shows a
/// question or a card; after that it stays `LINGER_MS` (The Breath in `done` for the first
/// `DONE_MS`, `justFinished`) and closes. `close()` tells the core (`voice_close`) and closes
/// at once; the stage opens again once the core has gone idle and is busy again, or listens.
export function useStage(act: StageCommands = commands): {
  open: boolean;
  justFinished: boolean;
  close: () => void;
} {
  const busy = useOpenDotBusy();
  const state = useVoiceLive((live) => live.state);

  // Changes seen while rendering (React's "adjust state when a prop changes"), so the stage
  // never draws one frame closed between busy and lingering, and Strict Mode's second effect
  // run cannot lose the linger.
  const [seen, setSeen] = useState({ busy, state });
  // The linger after the last busy moment: `id` restarts its timers, `done` is The Breath's
  // `done` beat.
  const [after, setAfter] = useState<{ id: number; done: boolean } | null>(null);
  const [closed, setClosed] = useState(false);
  if (seen.busy !== busy || seen.state !== state) {
    setSeen({ busy, state });
    if (seen.busy !== busy) {
      if (busy) {
        // Busy again after a quiet moment: whatever was closed before is over.
        setAfter(null);
        setClosed(false);
      } else {
        // The core caught up with a close, or Open Dot finished: linger unless closed.
        setAfter(closed ? null : { id: Date.now() + Math.random(), done: true });
        setClosed(false);
      }
    }
    // Listening again is a new conversation: a stage closed before it shows again.
    if (state === "listening" && seen.state !== "listening") setClosed(false);
  }

  const lingerId = after?.id;
  useEffect(() => {
    if (lingerId === undefined) return;
    const done = setTimeout(
      () => setAfter((a) => (a?.id === lingerId ? { ...a, done: false } : a)),
      DONE_MS,
    );
    const gone = setTimeout(() => setAfter((a) => (a?.id === lingerId ? null : a)), LINGER_MS);
    return () => {
      clearTimeout(done);
      clearTimeout(gone);
    };
  }, [lingerId]);

  const close = useCallback(() => {
    setClosed(true);
    setAfter(null);
    act.voiceClose().catch(notifyError);
  }, [act]);

  return {
    open: !closed && (busy || after !== null),
    justFinished: after?.done ?? false,
    close,
  };
}
