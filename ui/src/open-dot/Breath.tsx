import { useEffect, useId, useLayoutEffect, useRef, type JSX } from "react";

import { voiceStore } from "@/lib/use-voice";

import { createBreath, LOGO, type Bar, type BreathState } from "./breath-model";

/// The middle bar's colours, left to right across the whole box: lime, mint, sky.
const SWEEP = ["#c9f27a", "#5fe0a6", "#4bb8e8"] as const;

const LABELS: Record<BreathState, string> = {
  resting: "Sage is resting",
  done: "Sage is resting",
  listening: "Sage is listening",
  thinking: "Sage is thinking",
  speaking: "Sage is speaking",
  asking: "Sage is asking",
  trouble: "Sage has a problem",
};

/// How soft the glow behind the middle bar is. In the box's units, so it scales with `size` and
/// keeps the same proportion to the bars at every size.
const GLOW_BLUR = 12;
/// The rail form draws the arrangement this much larger around the middle, so the logo (100
/// units wide) fills about 80% of its box instead of half.
const FIT = 1.6;

export type BreathProps = {
  size: number;
  state: BreathState;
  options?: number;
  /// The rail form: still while resting unless the microphone is open, and no glow.
  quiet?: boolean;
  onClick?: () => void;
  label?: string;
};

/// A bar as the attributes of a `rect`. A bar narrower than it is tall would stop being a pill,
/// so one that is shrinking away is drawn as a circle that gets smaller.
function shape(bar: { x: number; y: number; w: number; h: number }) {
  const w = Math.max(0, bar.w);
  const h = Math.min(Math.max(0, bar.h), w);
  return {
    x: (bar.x - w / 2).toFixed(2),
    y: (bar.y - h / 2).toFixed(2),
    width: w.toFixed(2),
    height: h.toFixed(2),
    rx: (h / 2).toFixed(2),
  };
}
const SHAPE_ATTRIBUTES = ["x", "y", "width", "height", "rx"] as const;

/// The Breath: Sage's mark, three stacked bars that move with the person's voice and Sage's.
/// One animation loop writes the bars' attributes directly; nothing renders per frame. The
/// loop sleeps while nothing can move and wakes on a new state, options or voice change.
export function Breath({
  size,
  state,
  options = 3,
  quiet = false,
  onClick,
  label,
}: BreathProps): JSX.Element {
  const id = useId().replace(/[^A-Za-z0-9_-]/g, "");
  const sweepId = `breath-sweep-${id}`;
  const haloId = `breath-halo-${id}`;

  const group = useRef<SVGGElement>(null);
  const glow = useRef<SVGGElement>(null);
  const rects = useRef<(SVGRectElement | null)[]>([]);
  const halo = useRef<SVGRectElement>(null);
  // What the loop reads each frame; kept current without restarting it.
  const props = useRef({ state, options, quiet });
  // Starts the frame loop again after it went to sleep; set by the loop's effect.
  const wake = useRef<() => void>(() => {});
  useLayoutEffect(() => {
    props.current = { state, options, quiet };
    wake.current();
  }, [state, options, quiet]);

  useEffect(() => {
    const model = createBreath();
    const media =
      typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-reduced-motion: reduce)")
        : null;
    let still = media?.matches ?? false;

    // The last value written to each attribute, so a mark at rest touches nothing.
    const written = new Map<Element, Record<string, string>>();
    const put = (el: Element | null | undefined, name: string, value: string) => {
      if (!el) return;
      let seen = written.get(el);
      if (!seen) written.set(el, (seen = {}));
      if (seen[name] === value) return;
      seen[name] = value;
      el.setAttribute(name, value);
    };
    const place = (el: Element | null | undefined, bar: Bar | undefined) => {
      // The fourth bar has nothing to say while it is away; it keeps its place at no size.
      const s = shape(bar ?? { x: 0, y: 0, w: 0, h: 0 });
      if (!bar) return (["width", "height", "rx"] as const).forEach((name) => put(el, name, s[name]));
      for (const name of SHAPE_ATTRIBUTES) put(el, name, s[name]);
    };

    let handle: number | null = null;
    const draw = (now: number) => {
      handle = null;
      const live = voiceStore.get();
      const { state, options, quiet } = props.current;
      // The rail at rest holds the logo still unless the microphone is open.
      const shown = quiet && state === "resting" && !live.micOpen ? "done" : state;
      const frame = model.step(
        { state: shown, mic: live.level.mic, voice: live.level.voice, options, still },
        now,
      );
      for (let i = 0; i < 3; i++) place(rects.current[i], frame.bars[i as 0 | 1 | 2]);
      place(rects.current[3], frame.extra[0]);
      place(halo.current, frame.bars[1]);
      put(group.current, "opacity", frame.bars[0].opacity.toFixed(3));
      put(glow.current, "opacity", frame.glow.toFixed(3));
      // Nothing can move: every value is at its target (a held state, or any state with reduced
      // motion, once its morph is done), no sound is coming in or fading out, and the
      // microphone is shut. Sleep until the state, the options or the voice changes.
      const asleep =
        frame.settled && !live.micOpen && live.level.mic === 0 && live.level.voice === 0;
      if (!asleep) handle = window.requestAnimationFrame(draw);
    };
    const start = () => {
      if (handle === null && !document.hidden) handle = window.requestAnimationFrame(draw);
    };
    const stop = () => {
      if (handle !== null) window.cancelAnimationFrame(handle);
      handle = null;
    };
    wake.current = start;
    const onMedia = () => {
      still = media?.matches ?? false;
      start();
    };
    media?.addEventListener?.("change", onMedia);
    // The voice changed (the microphone, a level, anything): one frame tells whether to move.
    const unsubscribe = voiceStore.subscribe(start);
    const onVisibility = () => (document.hidden ? stop() : start());
    document.addEventListener("visibilitychange", onVisibility);
    start();
    return () => {
      stop();
      wake.current = () => {};
      unsubscribe();
      document.removeEventListener("visibilitychange", onVisibility);
      media?.removeEventListener?.("change", onMedia);
    };
  }, []);

  // Drawn in the logo arrangement; the loop takes over from the first frame. React never
  // changes these attributes again, as their values here never change.
  const start = [...LOGO, { ...LOGO[1], w: 0 }];
  const bars = (
    <g data-bars="" ref={group}>
      {start.map((bar, i) => (
        <rect
          key={i}
          {...(i < 3 ? { "data-bar": "" } : { "data-extra": "" })}
          ref={(el) => {
            rects.current[i] = el;
          }}
          {...shape(bar)}
          fill={i === 1 ? `url(#${sweepId})` : "currentColor"}
        />
      ))}
    </g>
  );
  const svg = (
    <svg
      width={size}
      height={size}
      viewBox="0 0 200 200"
      className="text-foreground"
      // The glow and a widening bar may reach past the box; the rail form has no glow.
      overflow={quiet ? undefined : "visible"}
      {...(onClick
        ? { "aria-hidden": true }
        : { role: "img", "aria-label": label ?? LABELS[state] })}
    >
      <defs>
        {/* Laid across the box, not the bar, so the colours stay put while the bar changes
            width and slides. */}
        <linearGradient id={sweepId} gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="200" y2="0">
          <stop offset="0" stopColor={SWEEP[0]} />
          <stop offset="0.5" stopColor={SWEEP[1]} />
          <stop offset="1" stopColor={SWEEP[2]} />
        </linearGradient>
        {!quiet && (
          // Sized in the box's units: a region measured from the bar would be too flat to hold
          // the blur above and below it.
          <filter id={haloId} filterUnits="userSpaceOnUse" x="-50" y="-50" width="300" height="300">
            <feGaussianBlur stdDeviation={GLOW_BLUR} />
          </filter>
        )}
      </defs>
      {!quiet && (
        <g data-glow="" ref={glow} filter={`url(#${haloId})`} opacity="0.28">
          <rect ref={halo} {...shape(LOGO[1])} fill={`url(#${sweepId})`} />
        </g>
      )}
      {quiet ? (
        <g data-fit="" transform={`translate(100 100) scale(${FIT}) translate(-100 -100)`}>
          {bars}
        </g>
      ) : (
        bars
      )}
    </svg>
  );

  if (!onClick) return svg;
  return (
    <button
      type="button"
      aria-label={label ?? "Stop and listen"}
      onClick={onClick}
      className="inline-flex shrink-0 cursor-pointer rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
    >
      {svg}
    </button>
  );
}
