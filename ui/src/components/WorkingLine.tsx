import { useEffect, useState } from "react";

/// What a dot is "doing" while it works on a reply: Dots' own playful phrases, shown one at a
/// time under the run so the person can see it is still going. Kept in one list so it is easy to
/// add to.
export const WORKING_PHRASES: readonly string[] = [
  "Connecting the sols",
  "Dotting the i's",
  "Drawing the lines",
  "Building bridges",
  "Laying the tracks",
  "Following the thread",
  "Untangling the threads",
  "Tying loose ends",
  "Piecing it together",
  "Sketching it out",
  "Hatching a plan",
  "Charting a course",
  "Mapping the maze",
  "Checking the compass",
  "Lining things up",
  "Stacking the blocks",
  "Sorting the pieces",
  "Shuffling index cards",
  "Flipping through pages",
  "Sharpening pencils",
  "Fetching the toolbox",
  "Warming up the gears",
  "Tuning the strings",
  "Polishing the edges",
  "Gathering the crumbs",
  "Counting the steps",
  "Spinning the yarn",
  "Reading between the lines",
  "Knitting it together",
  "Turning it over",
];

/// The pulsing dot: one frame every `FRAME_MS`, a dot that swells and settles.
export const DOT_FRAMES: readonly string[] = ["·", "•", "●", "◉", "●", "•"];
const FRAME_MS = 140;
/// How long each phrase stays before the next one.
export const PHRASE_MS = 3200;

/// A stable start in the list for a run, so two runs side by side don't say the same thing and a
/// re-render doesn't jump to another phrase.
export function firstPhrase(seed: string): number {
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) >>> 0;
  return h % WORKING_PHRASES.length;
}

/// Seconds as the line shows them: "12s", "1m 05s", "1h 02m".
export function elapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

function reducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/// The line a running reply shows: a pulsing dot, a phrase that changes every few seconds with a
/// shimmer across it, and how long the dot has been at it. Screen readers hear one steady
/// "<dot> is working" instead of every phrase. With reduced motion the dot holds still and the
/// shimmer stops; the phrase still changes, slowly.
export function WorkingLine(props: { seed: string; since: string | null; dotName?: string }) {
  const { seed, since } = props;
  const [frame, setFrame] = useState(0);
  const [phrase, setPhrase] = useState(() => firstPhrase(seed));
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const still = reducedMotion();
    const dot = still ? undefined : window.setInterval(() => setFrame((f) => f + 1), FRAME_MS);
    const words = window.setInterval(() => setPhrase((p) => p + 1), PHRASE_MS);
    const clock = window.setInterval(() => setNow(Date.now()), 1000);
    return () => {
      if (dot !== undefined) window.clearInterval(dot);
      window.clearInterval(words);
      window.clearInterval(clock);
    };
  }, []);

  const start = since ? Date.parse(since) : NaN;
  const seconds = Number.isNaN(start) ? null : (now - start) / 1000;
  return (
    <p role="status" data-slot="working" className="flex items-center gap-2 text-sm">
      <span className="sr-only">{props.dotName ?? "The sol"} is working</span>
      <span aria-hidden className="w-3 text-center text-primary">
        {DOT_FRAMES[frame % DOT_FRAMES.length]}
      </span>
      <span aria-hidden className="working-shimmer">
        {WORKING_PHRASES[phrase % WORKING_PHRASES.length]}…
      </span>
      {seconds !== null && (
        <span aria-hidden className="text-xs text-muted-foreground tabular-nums">
          {elapsed(seconds)}
        </span>
      )}
    </p>
  );
}
