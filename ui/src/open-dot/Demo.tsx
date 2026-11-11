// The Open Dot demo page, for development only (`/#/__open-dot` under `bun run dev`; not in a
// production build). It drives the stage and the rail mark by writing to `voiceStore` directly,
// answers their commands itself, and needs no backend, so it works in a plain browser.
import { QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode } from "react";

import type { OpenDotCard, Settings, SetupProposal, VoiceState, VoiceStatus } from "@/api/types";
import { Button } from "@/components/ui/button";
import { createQueryClient, keys } from "@/lib/query";
import { voiceStore, type VoiceLive } from "@/lib/use-voice";

import { OpenDotStage } from "./OpenDotStage";
import { RailMark } from "./RailMark";
import type { StageCommands } from "./use-stage";

const set = (change: Partial<VoiceLive>) => voiceStore.set(change);

/// Reply lines with running numbers, as the store keeps them.
let lastLine = 0;
const reply = (...texts: string[]) => texts.map((text) => ({ n: ++lastLine, text }));
/// One more line of the reply, the oldest dropped past three, as a `saying` event does.
const say = (text: string) =>
  set({ saying: text, said: [...voiceStore.get().said, ...reply(text)].slice(-3), hearing: "", heard: false });

const STATUS: VoiceStatus = {
  enabled: true,
  installed: true,
  downloading: false,
  ready: true,
  state: "idle",
  mic_open: false,
  download_bytes: 260_000_000,
  hotkey_error: null,
  trouble: null,
};

const PROPOSAL: SetupProposal = {
  name: "Downloads tidy",
  instructions:
    "Every weekday morning, read the Downloads folder.\nSort new files into Documents, Pictures and Installers.\nTell me what moved.",
  folder: "C:\\Users\\me\\Downloads",
  schedule: {
    rule: { kind: "weekly", days: ["mon", "tue", "wed", "thu", "fri"], time: "09:00" },
    catch_up: false,
    until: null,
  },
  preset: "sandboxed",
  extras: [],
  uses: { mcp_servers: [], skills: [], plugins: [], context: [], apps: [] },
  first_message: "Start with this week's files.",
  notes: "",
};

const LONG =
  "This is a long piece of text that goes on for a while, so the card has to scroll inside the panel rather than push the options out of the window. ";

const CARDS: [string, OpenDotCard][] = [
  ["Proposal", { kind: "proposal", proposal: PROPOSAL }],
  [
    "Proposal, long",
    {
      kind: "proposal",
      proposal: {
        ...PROPOSAL,
        name: "A dot with a very long name that keeps going and going",
        instructions: LONG.repeat(8),
        folder: "C:\\Users\\me\\Documents\\Projects\\a-folder-with-a-name-that-does-not-stop\\and-a-subfolder",
        schedule: { rule: { kind: "every", minutes: 120, anchor: null }, catch_up: true, until: { count: 5 } },
      },
    },
  ],
  [
    "Approve",
    {
      kind: "confirm",
      deed: { kind: "approve", approval_id: "a1", dot_name: "triage", tool: "Bash", target: "git push origin feat/open-dot" },
    },
  ],
  [
    "Deny",
    {
      kind: "confirm",
      deed: {
        kind: "deny",
        approval_id: "a2",
        dot_name: "triage",
        tool: "Write",
        target: "C:/Users/me/AppData/Roaming/SomeApp/configuration/a-very-long-file-name-without-spaces.json",
      },
    },
  ],
  ["Run", { kind: "confirm", deed: { kind: "run", dot_id: "d1", dot_name: "digest" } }],
  [
    "Send",
    {
      kind: "confirm",
      deed: { kind: "send", dot_id: "d1", dot_name: "digest", text: "Add Friday's meeting notes to the summary.", topic_root: null },
    },
  ],
  [
    "Answer",
    { kind: "confirm", deed: { kind: "answer", message_id: "m1", dot_name: "digest", text: "Yes, go ahead and send it." } },
  ],
  ["List", { kind: "list", title: "Waiting for you", lines: ["One approval from triage", "Two notes from digest"] }],
  [
    "List, long",
    { kind: "list", title: "Everything today", lines: Array.from({ length: 30 }, (_, i) => `Item ${i + 1}: ${LONG.slice(0, 60 + i * 3)}`) },
  ],
];

const ASKS: Record<number, string[]> = {
  2: ["Yes", "Not now"],
  3: ["Allow", "Deny", "Ask me later"],
  4: ["Morning", "Afternoon", "Evening", "Never mind"],
  6: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Every weekday at nine in the morning"],
};

function Group(props: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-2 rounded-lg border p-3">
      <legend className="px-1 text-xs font-medium text-muted-foreground">{props.title}</legend>
      <div className="flex flex-wrap gap-1.5">{props.children}</div>
    </fieldset>
  );
}

function B(props: { onClick: () => void; children: ReactNode; pressed?: boolean }) {
  return (
    <Button type="button" size="xs" variant={props.pressed ? "default" : "outline"} aria-pressed={props.pressed} onClick={props.onClick}>
      {props.children}
    </Button>
  );
}

/// A level that rises and falls like speech: syllables on a slower phrase swell, with pauses.
function speechLevel(t: number): number {
  const phrase = Math.max(0, Math.sin(t * 1.3));
  const syllable = 0.55 + 0.45 * Math.sin(t * 9.1) * Math.sin(t * 5.3 + 1);
  return Math.min(1, Math.max(0, phrase * syllable + 0.04 * Math.sin(t * 23)));
}

export default function Demo() {
  const [qc] = useState(() => {
    const client = createQueryClient();
    client.setQueryData(keys.voice, STATUS);
    // Only the hotkey is read here (the rail mark's name); the rest of the settings is not.
    client.setQueryData(keys.settings, { voice_hotkey: "CmdOrCtrl+Shift+Space" } as Settings);
    return client;
  });
  const live = useSyncExternalStore(voiceStore.subscribe, voiceStore.get);
  const [ready, setReady] = useState(true);
  const [talking, setTalking] = useState(false);
  const [controls, setControls] = useState(true);
  const [log, setLog] = useState<string[]>([]);
  const timers = useRef<number[]>([]);
  const said = (line: string) => setLog((l) => [line, ...l].slice(0, 6));

  useEffect(() => {
    voiceStore.reset();
    set({ state: "idle" });
    const pending = timers.current;
    return () => {
      for (const t of pending) clearTimeout(t);
      voiceStore.reset();
    };
  }, []);

  useEffect(() => {
    qc.setQueryData<VoiceStatus>(keys.voice, (s) => (s ? { ...s, ready, enabled: ready } : s));
  }, [qc, ready]);

  // "Talk": a smoothly varying level, into the microphone while listening and the voice otherwise.
  useEffect(() => {
    if (!talking) return;
    const start = performance.now();
    const id = window.setInterval(() => {
      const level = speechLevel((performance.now() - start) / 1000);
      const listening = voiceStore.get().state === "listening";
      set({ level: listening ? { mic: level, voice: 0 } : { mic: 0, voice: level } });
    }, 33);
    return () => {
      clearInterval(id);
      set({ level: { mic: 0, voice: 0 } });
    };
  }, [talking]);

  const later = (ms: number, f: () => void) => timers.current.push(window.setTimeout(f, ms));
  const stopScript = () => {
    for (const t of timers.current) clearTimeout(t);
    timers.current = [];
  };

  const act: StageCommands = {
    voiceOpen: async () => {
      said("voice_open");
      set({ state: "listening", micOpen: true, trouble: null });
    },
    voiceClose: async () => {
      said("voice_close");
      stopScript();
      setTalking(false);
      set({ state: "idle", micOpen: false, hearing: "", heard: false, saying: "", said: [], ask: null, card: null });
    },
    voiceStop: async () => {
      said("voice_stop");
      set({ state: "listening", micOpen: true, trouble: null });
    },
    voicePick: async (index) => {
      said(`voice_pick(${index})`);
      later(500, () => set({ state: "thinking", micOpen: false, ask: null, card: null }));
      later(1500, () => set({ state: "speaking", said: reply("Done.") }));
      later(2500, () => set({ state: "idle" }));
    },
  };

  const state = (s: VoiceState) =>
    set({ state: s, micOpen: s === "listening" || s === "asking", ...(s === "listening" ? { trouble: null } : {}) });

  const playTurn = () => {
    stopScript();
    set({ hearing: "", heard: false, saying: "", said: [], ask: null, card: null, trouble: null });
    state("listening");
    setTalking(true);
    const words = ["what's", "what's waiting", "what's waiting for", "what's waiting for me"];
    words.forEach((w, i) => later(300 + i * 450, () => set({ hearing: w, heard: false })));
    later(2200, () => {
      setTalking(false);
      set({ state: "thinking", micOpen: false, hearing: "what's waiting for me?", heard: true, said: [] });
    });
    later(3600, () => {
      set({ state: "speaking" });
      say("Triage wants to push a branch.");
      setTalking(true);
    });
    later(5800, () => say("It is the open dot branch, to origin."));
    later(8000, () => {
      setTalking(false);
      // The question is spoken as a reply line too: the stage prints it once, above the options.
      say("Allow triage to push?");
      set({
        state: "asking",
        micOpen: true,
        ask: { question: "Allow triage to push?", options: ["Allow", "Deny", "Not now"] },
        card: CARDS[2]![1],
      });
    });
    later(11500, () => set({ state: "thinking", micOpen: false, ask: null, card: null }));
    later(12500, () => set({ state: "speaking", said: reply("Allowed. It is pushing now.") }));
    later(14000, () => set({ state: "idle", micOpen: false }));
  };

  const reduced = typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  return (
    <QueryClientProvider client={qc}>
      <div className="flex h-screen bg-background text-sm text-foreground">
        <nav aria-label="Demo rail" className="flex w-[46px] shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-2">
          <div className="mt-auto">
            <RailMark act={act} />
          </div>
        </nav>
        <main className="min-w-0 flex-1 overflow-auto p-4">
          {/* Above the stage, so the controls stay usable while it shows; hide them for a clean
              look at the stage. */}
          <Button
            type="button"
            size="xs"
            variant="secondary"
            className="fixed top-2 right-2 z-[60]"
            aria-expanded={controls}
            onClick={() => setControls((c) => !c)}
          >
            {controls ? "Hide controls" : "Show controls"}
          </Button>
          <div className={controls ? "relative z-50 flex max-w-3xl flex-col gap-3 rounded-xl bg-background/95" : "hidden"}>
            <div>
              <h1 className="text-base font-semibold">Open Dot demo</h1>
              <p className="text-xs text-muted-foreground">
                Development only. Writes to the voice store directly; the commands are answered here. Now:{" "}
                <span className="font-mono">
                  {live.state}
                  {live.micOpen ? " · mic open" : ""}
                </span>
                {log.length > 0 && <span className="font-mono"> · {log.join(" · ")}</span>}
              </p>
            </div>
            <Group title="State">
              {(["idle", "listening", "thinking", "speaking", "asking", "off"] as VoiceState[]).map((s) => (
                <B key={s} pressed={live.state === s} onClick={() => state(s)}>
                  {s}
                </B>
              ))}
              <B pressed={live.micOpen} onClick={() => set({ micOpen: !live.micOpen })}>
                Mic open
              </B>
              <B pressed={ready} onClick={() => setReady((r) => !r)}>
                Voice ready
              </B>
            </Group>
            <Group title="Level">
              <label className="flex items-center gap-2 text-xs">
                Level
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.01}
                  className="w-40 accent-primary"
                  value={Math.max(live.level.mic, live.level.voice)}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    set({ level: { mic: v, voice: v } });
                  }}
                />
              </label>
              <B pressed={talking} onClick={() => setTalking((t) => !t)}>
                Talk
              </B>
            </Group>
            <Group title="Words">
              <B onClick={() => set({ hearing: "what is waiting for me", heard: false })}>Hearing</B>
              <B onClick={() => set({ hearing: "what is waiting for me today?", heard: true })}>Heard</B>
              <B onClick={() => say(`Reply line ${lastLine + 1}.`)}>Reply line</B>
              <B onClick={() => say(LONG.repeat(3))}>Long reply line</B>
              <B onClick={() => set({ hearing: "", heard: false, saying: "", said: [], ask: null, card: null })}>
                Cleared
              </B>
            </Group>
            <Group title="Ask">
              {[2, 3, 4, 6].map((n) => (
                <B key={n} onClick={() => set({ state: "asking", micOpen: true, ask: { question: `Pick one of ${n}`, options: ASKS[n]! } })}>
                  {n} options
                </B>
              ))}
              <B onClick={() => set({ ask: { question: "Long options", options: [LONG, LONG.slice(0, 80)] } })}>Long options</B>
              <B
                onClick={() => {
                  say("What should the new dot be called?");
                  set({ state: "asking", micOpen: true, ask: { question: "What should the new dot be called?", options: [] } });
                }}
              >
                Open question
              </B>
              <B onClick={() => set({ ask: { question: LONG.repeat(2).trim() + "?", options: ASKS[3]! } })}>Long question</B>
            </Group>
            <Group title="Card">
              {CARDS.map(([name, card]) => (
                <B key={name} onClick={() => set({ card })}>
                  {name}
                </B>
              ))}
            </Group>
            <Group title="Other">
              <B onClick={() => set({ trouble: "No microphone was found. Plug one in or choose another in Settings.", micOpen: false })}>
                Trouble
              </B>
              <B onClick={() => set({ trouble: null })}>No trouble</B>
              <B onClick={playTurn}>Play a turn</B>
            </Group>
            <p className="text-xs text-muted-foreground">
              Reduced motion is {reduced ? "on" : "off"}. To try it, turn on reduced motion in the system settings or
              emulate <span className="font-mono">prefers-reduced-motion: reduce</span> in the browser's developer tools,
              then reload: the stage only fades in, and The Breath holds the logo and changes only its brightness.
            </p>
          </div>
        </main>
        <OpenDotStage act={act} />
      </div>
    </QueryClientProvider>
  );
}
