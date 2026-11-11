# Open Dot Step 4: The Stage and The Breath

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** What the person sees when they talk to Dot: a panel that rises from the bottom centre of the window with The Breath (our own animated mark: the three dots of the Dots logo as soft bodies that breathe, lean, circle, melt together and part), the live words, the options, and a card; plus the small mark at the bottom of the rail that starts it.

**Architecture:** UI only. A pure model (`breath-model.ts`) turns state, sound level and time into a few numbers per dot; `Breath.tsx` draws them as SVG circles under a blur-and-threshold filter, driven by `requestAnimationFrame` and reading the level straight from `voiceStore` (no React render per frame). `OpenDotStage.tsx` reads the voice store through selectors and sends `voice_*` commands. A dev-only demo route lets the stage be seen and screenshotted with a fake store.

**Tech Stack:** React 18, TypeScript, Tailwind (existing tokens), vitest + Testing Library. No animation library, no new dependency.

**Spec:** `docs/superpowers/specs/2026-11-11-open-dot-design.md` sections 3.2, 3.3, 3.4, 3.6, 4, 7.

**Prototype to port:** `.superpowers/sdd/2026-11-11-open-dot-4-stage/the-breath.html` (a working single-file version of The Breath with every state; the motion there was accepted as the direction, so port its numbers rather than inventing new ones).

## Global Constraints

- The Breath is the three logo dots: yellow `#f2c94c`, blue `#56ccf2`, green `#6fcf97`, on the app's deep teal surface. Not an orb, not a waveform, not a ring.
- States and what they do (spec section 4): Resting (slow swell, about one breath every 4.5 s, each dot slightly out of step), Listening (lean apart, each swells with the person's voice, yellow most), Thinking (circle one another slowly, touching and parting), Speaking (melt into one shape that pulses with Dot's voice, colours still visible), Asking (part into as many lobes as there are options, hold), Done (one quick settle to the logo arrangement), Trouble (dim and still; no shaking, no red).
- Motion follows the real sound level whenever sound is present, never a canned loop. Every change between states is a continuous morph of 250 to 400 ms, never a cut.
- With "reduce motion" on (`prefers-reduced-motion: reduce`): the dots hold still and only brightness changes.
- Built with SVG and a blur-and-threshold filter driven by a few numbers per frame. No animation library.
- `Level` events arrive up to 30 a second: nothing outside The Breath may re-render because of them.
- The stage lives in the main window above whatever screen is showing; it never blocks the screen behind it from being read (no full-screen scrim) and never steals focus from a text field the person is typing in.
- `Esc` closes the stage and stops speech (`voice_close`), unless a dialog or menu is open (those keep their own `Esc`). Clicking The Breath stops speech and listens (`voice_stop`). Options are picked by click or by keys `1` to `4` (`voice_pick(index)`), never by sending text.
- Whenever the microphone is open the rail mark shows it. When the microphone is not open while the stage is, the stage says why in one line.
- Copy is plain and short. No jargon ("session", "turn", "pending"). Accessible: the stage is a labelled region, live words are an `aria-live="polite"` area, options are buttons, everything reachable by keyboard.
- Follow the UI's patterns: Tailwind tokens already in use, `lucide` icons, Geist; tests beside the code with vitest; `bun run typecheck`, `bun run test`, `bun run build` pass.
- Commits: conventional messages, no co-author trailer, staged by path. Touch only `ui/` (and the spec in the last task).

## What exists (built in step 3)

- `ui/src/lib/use-voice.ts`: `voiceStore` (`get`, `set`, `subscribe`), `useVoiceLive(select)`, `useVoiceStatus()`, `VoiceLive { state, micOpen, level: {mic, voice}, hearing, saying, ask, card, download, trouble }`, `useVoice()` mounted in `Layout`.
- `ui/src/api/commands.ts`: `voiceOpen`, `voiceClose`, `voiceStop`, `voicePick(index)`, `voiceStatus`, and the rest. `ui/src/api/types.ts`: `VoiceState` (`off | idle | listening | thinking | speaking | asking`), `OpenDotCard` (`proposal { proposal }`, `confirm { deed }`, `list { title, lines }`), `Deed`.
- `ui/src/app/Layout.tsx`: the rail (`<nav aria-label="Main">`, `LINKS`, the Search button, `LimitsMeter` last), the only global key listener (Ctrl/Cmd+K).
- Known gap to close here: `VoiceSection` has its own status query that can overwrite a newer `status` event; and the hook keeps `saying` as the last sentence only.

## Review Focus

1. A level event must not re-render the stage, the rail or the layout (profiler test).
2. Reduced motion: positions and sizes are constant across frames and states; only opacity changes (model test).
3. `Esc` with a dialog open closes the dialog, not the stage; typing `1` in a text field does not pick option 1.
4. The stage with long content: a 400-byte question, six options, a proposal with long instructions: nothing overflows the window at 800 × 560, the options stay reachable.
5. Voice off or not ready: clicking the rail mark never opens a dead stage; it takes the person to Settings, Voice.

---

### Task 1: The Breath's model

**Files:** Create `ui/src/open-dot/breath-model.ts`, `ui/src/open-dot/breath-model.test.ts`.

**Interfaces — Produces:**

```ts
export type BreathState = "resting" | "listening" | "thinking" | "speaking" | "asking" | "done" | "trouble";
export type Dot = { x: number; y: number; r: number; opacity: number };          // in a 200 × 200 box
export type Frame = { dots: [Dot, Dot, Dot]; lobes: Dot[]; glow: number };       // lobes: extra bodies while asking with 4+ options
export type BreathInput = { state: BreathState; mic: number; voice: number; options: number; still: boolean };
export function createBreath(): { step(input: BreathInput, nowMs: number): Frame };
export function breathStateOf(live: { state: VoiceState; trouble: string | null; ask: unknown | null }, justFinished: boolean): BreathState;
export const MORPH_MS = 320;
```

- `createBreath` holds the smoothed values between frames (it is the only stateful part); `step` is called once per animation frame with the wall clock. Port the targets and easing from the prototype (`targets(t)`, `voice(dt)`, `frame(now)`): logo arrangement, lean distances, circling radius and speed, melt positions, lobe placement, level smoothing (fast attack, slower release).
- State changes ease from the current values to the new targets over `MORPH_MS` (250 to 400 allowed); no value jumps by more than what one frame of easing allows.
- `still: true`: `x`, `y`, `r` equal the logo arrangement in every state; only `opacity` and `glow` vary (listening and speaking brighten with the level; trouble dims).
- `breathStateOf`: `off`/`idle` to `resting` (or `done` for about 600 ms after a reply ends), `listening`, `thinking`, `speaking`, `asking` (or when `ask` is set while idle); any `trouble` to `trouble`.

- [ ] Tests on numbers, not pixels: `resting_breathes_about_every_4_5_seconds` (radius period between 4.2 and 4.8 s, dots out of phase); `listening_swells_with_the_mic_and_yellow_most`; `silence_brings_listening_back_to_a_shallow_breath`; `thinking_circles` (centres move on a loop, pairs come within touching distance and part); `speaking_melts_and_pulses_with_voice` (centres within one radius; size follows `voice`); `asking_parts_into_as_many_lobes_as_options` (2, 3, 4 options; capped at 4 bodies, more options keep 4); `done_settles_to_the_logo`; `trouble_dims_and_holds_still`; `state_changes_are_continuous` (no per-frame jump above a bound across every pair of states); `reduced_motion_only_changes_brightness` (**Review Focus 2**); `level_is_smoothed` (a spike does not jump the radius); `survives_bad_input` (NaN and out-of-range levels are treated as 0 to 1).
- [ ] Commit `feat(ui): the numbers behind The Breath`.

### Task 2: The Breath

**Files:** Create `ui/src/open-dot/Breath.tsx`, `ui/src/open-dot/Breath.test.tsx`.

**Interfaces — Produces:** `export function Breath(props: { size: number; state: BreathState; options?: number; quiet?: boolean; onClick?: () => void; label?: string }): JSX.Element` — `quiet` is the rail form: resting stays perfectly still unless the microphone is open, and no glow.

- SVG with a unique filter id per instance (`useId`): `feGaussianBlur` then `feColorMatrix` threshold as in the prototype (blur scales with `size`), three coloured circles plus lobes, a soft glow layer behind.
- One `requestAnimationFrame` loop per mounted Breath; each frame reads `voiceStore.get().level` (no subscription), calls `step`, and writes `cx`, `cy`, `r`, `opacity` on the circle elements through refs. No React state changes per frame. The loop stops when the document is hidden and when unmounted. `still` comes from `matchMedia("(prefers-reduced-motion: reduce)")` and follows changes.
- When `onClick` is given the mark is a `button` with `aria-label` (default "Stop and listen"); otherwise `role="img"` with `aria-label` naming the state ("Dot is listening", "Dot is thinking", "Dot is speaking", "Dot is asking", "Dot is resting", "Dot has a problem").
- [ ] Tests: renders three circles in the logo colours; a frame writes attributes without re-rendering (render count stays 1 across 30 store level changes and 10 frames, using a fake `requestAnimationFrame`); stops the loop on unmount and when hidden; `quiet` and resting writes the same numbers every frame; reduced motion honoured; button vs image roles and labels.
- [ ] Commit `feat(ui): The Breath, three dots that move with the voice`.

### Task 3: The stage

**Files:** Create `ui/src/open-dot/OpenDotStage.tsx`, `ui/src/open-dot/OpenDotStage.test.tsx`, `ui/src/open-dot/cards.tsx`, `ui/src/open-dot/use-stage.ts`; modify `ui/src/lib/use-voice.ts` (see below) and `ui/src/screens/VoiceSection.tsx` (status only through the hook's cache).

- `use-voice.ts`: add `said: string[]` to `VoiceLive` (the lines of the current reply, appended on each `saying`, cleared when a new `heard` arrives or on `cleared`; keep at most the last 3); set `hearing` to `""` when a reply starts; make `useVoiceStatus` the only reader of `keys.voice` (no second fetch that can overwrite a newer event: `staleTime: Infinity`, seeded and kept by the hook), and have `VoiceSection` use it.
- `useStage()`: the stage is open when `state` is `listening`, `thinking`, `speaking` or `asking`, or an `ask` or `card` is showing; after the state returns to `idle` with nothing showing it stays for 2.5 s (showing the last words, The Breath in `done` then `resting`) and then closes; `close()` calls `voiceClose()` and closes at once.
- Layout of the panel (bottom centre, `max-w-xl`, rises with a 250 ms translate-and-fade; with reduced motion it only fades): The Breath (size 132, clickable: `voiceStop()`), under it the live words (what it hears in muted type while listening; then the reply lines, newest strongest; two lines at most, the rest truncated with the full text in `title`), then options when asking (pills numbered 1 to 4 with the option text; more than four options wrap and only the first four have key hints), then a card, then a footer row: a close button ("Close", `Esc`) and, when the microphone is not open while the stage is, one line: thinking "Thinking…", speaking "Speaking. Click the dots or press the shortcut to interrupt.", idle with something showing "Not listening. Click the dots to answer by voice, or pick an option.", trouble: the trouble sentence.
- Cards (`cards.tsx`): `proposal`: name, what it will do (instructions, clamped to 6 lines with "Show all"), schedule in words (reuse the app's existing schedule formatter), folder, first message; `confirm`: by deed kind — approve/deny: dot name, tool, target in monospace, wrapped; run: dot name; send/answer: dot name and the text; `list`: title and lines. Cards are read-only; the decision is made with the options.
- Keys (one listener, added while the stage is open): `Esc` closes unless an element with `role="dialog"` or `role="menu"` is open or the event was already handled; `1` to `4` pick an option only when options are showing and the focus is not in an input, textarea, select or contenteditable (**Review Focus 3**). Focus is not moved into the stage when it opens; `Tab` reaches it in order.
- `aria`: the panel is `role="region"` `aria-label="Open Dot"`; live words `aria-live="polite"`; options `role="group"` `aria-label` = the question.
- [ ] Tests: opens on listening, shows hearing text; shows reply lines; asking shows numbered options, click calls `voice_pick` with the index, key `2` picks index 1, key `2` while typing in an input does nothing; `Esc` calls `voice_close`; `Esc` with a dialog open does not; clicking The Breath calls `voice_stop`; each card kind renders its fields; lingers 2.5 s after idle then closes (fake timers); the mic-off line per state; a level event does not re-render the stage (**Review Focus 1**, profiler); long content stays inside the panel (class assertions for clamp/wrap/scroll on the card area with `max-h` and `overflow-y-auto`) (**Review Focus 4**); `cleared` removes options and card.
- [ ] Commit `feat(ui): the Open Dot stage: live words, options and a card around The Breath`.

### Task 4: The rail mark, mounting, and a demo route

**Files:** Modify `ui/src/app/Layout.tsx`, `ui/src/app/Layout.test.tsx`, `ui/src/app/routes.tsx`; create `ui/src/open-dot/RailMark.tsx`, `ui/src/open-dot/RailMark.test.tsx`, `ui/src/open-dot/Demo.tsx`.

- `RailMark`: a 36 px rail button at the bottom of the rail, above `LimitsMeter`, holding `<Breath size={24} quiet />`; `aria-label` "Talk to Dot" plus the shortcut from settings when one is set. Click: when voice is ready, `voiceOpen()` (while the stage is open it acts as stop-and-listen, same as the hotkey); when voice is off, not installed or still getting ready, navigate to `/settings/voice` (**Review Focus 5**). While the microphone is open the mark breathes and has a small ring in the primary colour; a tooltip says "Dot is listening".
- `Layout` renders `<OpenDotStage />` once, after the main content, so it floats above every screen.
- `Demo` (route `/__open-dot`, registered only when `import.meta.env.DEV`): a page that mounts the stage and the rail mark with buttons to push each state, levels (a slider and a "talk" button that feeds a varying level), an ask with 2, 3, 4 and 6 options, each card kind, long content, trouble, and a "play a turn" script; it writes to `voiceStore` directly and stubs the commands it calls. It must not be in the production bundle (assert in a test that the route list has no `/__open-dot` when `DEV` is false, or check the built chunk names).
- [ ] Tests: rail mark click when ready calls `voice_open`; when off navigates to settings voice; shows the ring when the mic is open; the layout shows the stage only when open; a level event does not re-render `Layout`.
- [ ] Commit `feat(ui): a mark at the bottom of the rail talks to Dot, and the stage floats above every screen`.

### Task 5: Look at it, gate, as built

- [ ] Run the UI dev server (`bun run --cwd ui dev`, Vite on 5173; it needs no Tauri for the demo route — if the app's entry requires Tauri APIs at boot, make the demo route reachable under the test mocks or a tiny dev shim, and say what you did) and capture screenshots of `/__open-dot` at 1200 × 800 and 800 × 560 in each state (resting, listening with level, thinking, speaking, asking with 2 and 4 options, each card, trouble, reduced motion), in light and dark theme if the app has both. Save them under `.superpowers/sdd/2026-11-11-open-dot-4-stage/shots/`. Fix what looks wrong (overflow, contrast below 4.5:1 for text, clipped glow, misaligned pills) and re-capture.
- [ ] Gate: `bun run typecheck`, `bun run test`, `bun run build` in `ui`; `cargo test -p dots-app --locked contract` unchanged (no API change).
- [ ] Spec section 10: "### Step 4: the stage and The Breath" (what was built, differences from sections 3.3 and 4 with reasons, deferred items, what is owed: the user's look at the real thing with a real voice). Status line updated. Commit `docs(spec): Open Dot step 4 as built`.
