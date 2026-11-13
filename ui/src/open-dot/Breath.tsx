import { useEffect, useId, useLayoutEffect, useRef, type JSX } from "react";

import { voiceStore } from "@/lib/use-voice";

import { createBreath, LOGO, type BreathState } from "./breath-model";

/// The three dots of the Dots mark: yellow, blue, green. The fourth body is the extra lobe for a
/// fourth option, in the app's teal.
const COLOURS = ["#f2c94c", "#56ccf2", "#6fcf97", "#22c3a6"] as const;

const LABELS: Record<BreathState, string> = {
  resting: "Sage is resting",
  done: "Sage is resting",
  listening: "Sage is listening",
  thinking: "Sage is thinking",
  speaking: "Sage is speaking",
  asking: "Sage is asking",
  trouble: "Sage has a problem",
};

/// The prototype's blurs (9 and 26 in its units), in the 200 box. They are in the box's units,
/// so they scale with `size` and keep the same proportion to the dots at every size.
const MELT_BLUR = 9 * 0.7;
const HALO_BLUR = 26 * 0.7;
/// Below this many pixels the dots are drawn without melting. The melt blurs the dots and cuts
/// the blur at a hard edge: at 24 px the small dot is about 2 px across in radius and the blur
/// under 1 px, so the cut leaves jagged specks with no antialiasing. Plain circles read better.
const MELT_MIN_SIZE = 40;
/// The melt's alpha ramp: a soft edge (12) cut at the prototype's point (alpha 5/12, as 24 -10).
const MELT_ALPHA = "0 0 0 12 -5";
/// The rail form draws the arrangement this much larger around the middle, so the logo fills
/// about 80% of its box instead of 56%.
const FIT = 1.45;

export type BreathProps = {
  size: number;
  state: BreathState;
  options?: number;
  /// The rail form: still while resting unless the microphone is open, and no glow.
  quiet?: boolean;
  onClick?: () => void;
  label?: string;
};

/// The Breath: Open Dot's mark, three soft dots that move with the person's voice and Dot's.
/// One animation loop writes the circles' attributes directly; nothing renders per frame. The
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
  const meltId = `breath-melt-${id}`;
  const haloId = `breath-halo-${id}`;
  const melts = size >= MELT_MIN_SIZE;

  const bodies = useRef<SVGGElement>(null);
  const glow = useRef<SVGGElement>(null);
  const circles = useRef<(SVGCircleElement | null)[]>([]);
  const halos = useRef<(SVGCircleElement | null)[]>([]);
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
      for (let i = 0; i < 4; i++) {
        const dot = i < 3 ? frame.dots[i as 0 | 1 | 2] : frame.lobes[0];
        const cx = (dot?.x ?? 0).toFixed(2);
        const cy = (dot?.y ?? 0).toFixed(2);
        const r = Math.max(0, dot?.r ?? 0).toFixed(2);
        for (const el of [circles.current[i], halos.current[i]]) {
          put(el, "cx", cx);
          put(el, "cy", cy);
          put(el, "r", r);
        }
      }
      put(bodies.current, "opacity", frame.dots[0].opacity.toFixed(3));
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
  const start = [...LOGO, { x: LOGO[0].x, y: LOGO[0].y, r: 0 }];
  const body = (
    <g data-bodies="" ref={bodies} filter={melts ? `url(#${meltId})` : undefined}>
      {start.map((d, i) => (
        <circle
          key={i}
          {...(i < 3 ? { "data-dot": "" } : { "data-lobe": "" })}
          ref={(el) => {
            circles.current[i] = el;
          }}
          cx={d.x.toFixed(2)}
          cy={d.y.toFixed(2)}
          r={d.r.toFixed(2)}
          fill={COLOURS[i]}
        />
      ))}
    </g>
  );
  const svg = (
    <svg
      width={size}
      height={size}
      viewBox="0 0 200 200"
      // The glow and a swelling shape may reach past the box; the rail form has no glow.
      overflow={quiet ? undefined : "visible"}
      {...(onClick
        ? { "aria-hidden": true }
        : { role: "img", "aria-label": label ?? LABELS[state] })}
    >
      <defs>
        {melts && (
          <filter
            id={meltId}
            x="-30%"
            y="-30%"
            width="160%"
            height="160%"
            colorInterpolationFilters="sRGB"
          >
            <feGaussianBlur in="SourceGraphic" stdDeviation={MELT_BLUR} result="blur" />
            <feColorMatrix
              in="blur"
              mode="matrix"
              values={`1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  ${MELT_ALPHA}`}
            />
          </filter>
        )}
        {!quiet && (
          <filter id={haloId} x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation={HALO_BLUR} />
          </filter>
        )}
      </defs>
      {!quiet && (
        <g data-glow="" ref={glow} filter={`url(#${haloId})`} opacity="0.28">
          {start.map((d, i) => (
            <circle
              key={i}
              ref={(el) => {
                halos.current[i] = el;
              }}
              cx={d.x.toFixed(2)}
              cy={d.y.toFixed(2)}
              r={d.r.toFixed(2)}
              fill={COLOURS[i]}
            />
          ))}
        </g>
      )}
      {quiet ? <g data-fit="" transform={`translate(100 100) scale(${FIT}) translate(-100 -100)`}>{body}</g> : body}
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
