import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useSyncExternalStore } from "react";
import { matchPath, useLocation, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import { onVoice } from "@/api/events";
import type {
  OpenDotCard,
  OpenTarget,
  Settings,
  View,
  VoiceEvent,
  VoiceState,
  VoiceStatus,
} from "@/api/types";
import { invalidate, keys } from "@/lib/query";
import { useCurrentScope } from "@/lib/scope";

/// What Open Dot is doing from moment to moment. Kept out of the query cache: a level arrives
/// up to 30 times a second, and only what reads it should draw again.
export type VoiceLive = {
  state: VoiceState;
  micOpen: boolean;
  level: { mic: number; voice: number };
  /// What the person is saying, as it is heard; the whole of it once heard.
  hearing: string;
  /// Whether `hearing` is the finished sentence (a `heard` event), not words still coming.
  heard: boolean;
  saying: string;
  /// The lines of the current reply, oldest first: the last `SAID_KEPT` of them. `n` is the
  /// line's running number, never reused, so a line keeps its name as older ones drop off.
  said: SaidLine[];
  /// `seq` names the question: a pick on its options sends it back.
  ask: { question: string; options: string[]; seq: number } | null;
  card: OpenDotCard | null;
  /// The speech files' download, while one runs.
  download: { done: number; total: number } | null;
  trouble: string | null;
  /// How many `trouble` events came: the stage opens on each new one, so a problem is seen
  /// even when nothing else changes (a microphone that will not open leaves Dot resting).
  troubles: number;
};

export type SaidLine = { n: number; text: string };

const quiet: VoiceLive = {
  state: "off",
  micOpen: false,
  level: { mic: 0, voice: 0 },
  hearing: "",
  heard: false,
  saying: "",
  said: [],
  ask: null,
  card: null,
  download: null,
  trouble: null,
  troubles: 0,
};

function createStore() {
  let live = quiet;
  const listeners = new Set<() => void>();
  return {
    get: () => live,
    set(change: Partial<VoiceLive>) {
      live = { ...live, ...change };
      for (const l of listeners) l();
    },
    reset() {
      live = quiet;
      for (const l of listeners) l();
    },
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
  };
}

/// The one store of the app's voice: `useVoice` writes it, `useVoiceLive` reads it.
export const voiceStore = createStore();

/// One part of what Open Dot is doing. A component draws again only when `select` returns
/// something else, so `select` returns a field, not a new object.
export function useVoiceLive<T>(select: (live: VoiceLive) => T): T {
  return useSyncExternalStore(voiceStore.subscribe, () => select(voiceStore.get()));
}

/// How many lines of a reply the store keeps.
const SAID_KEPT = 3;

/// The running number of the last reply line taken, over the app's life.
let lastLine = 0;

/// How many times the voice status was set from something newer than a read (an event, or a
/// command's answer), per query client. A read that began before such a change keeps the newer
/// status instead of its own answer.
const changes = new WeakMap<QueryClient, number>();
const changesOf = (qc: QueryClient) => changes.get(qc) ?? 0;

/// Takes a voice status newer than any read on its way: a `status` event, or the answer of a
/// command that changed it.
export function takeVoiceStatus(qc: QueryClient, status: VoiceStatus): void {
  changes.set(qc, changesOf(qc) + 1);
  qc.setQueryData(keys.voice, status);
}

/// Reads the voice status, unless a newer one arrived while the read was on its way.
async function readVoiceStatus(qc: QueryClient): Promise<VoiceStatus> {
  const before = changesOf(qc);
  const status = await commands.voiceStatus();
  if (changesOf(qc) === before) return status;
  return qc.getQueryData<VoiceStatus>(keys.voice) ?? status;
}

const statusQuery = (qc: QueryClient) => ({
  queryKey: keys.voice,
  queryFn: () => readVoiceStatus(qc),
  staleTime: Infinity,
});

/// The voice status: the one reader of `keys.voice`. `useVoice` seeds it and keeps it from
/// events; a read never replaces a status that arrived while it was on its way.
export function useVoiceStatus() {
  const qc = useQueryClient();
  return useQuery(statusQuery(qc));
}

/// Where an `open` event takes the person (app/routes.tsx). The Inbox has no address per
/// approval, so an approval opens the Inbox.
export function pathOf(target: OpenTarget): string {
  switch (target.kind) {
    case "dot":
      return `/home/${encodeURIComponent(target.dot_id)}`;
    case "approval":
    case "inbox":
      return "/inbox";
    case "settings":
      return "/settings";
    case "usage":
      return "/usage";
    case "home":
      return "/home";
  }
}

/// What the screen at `pathname` shows, for Open Dot: a short name, the dot, and the topic
/// when its address names one. The current topic of a conversation opened without one is not
/// in the address, so it is not named.
export function viewOf(pathname: string): Pick<View, "screen" | "dot_id" | "topic_root"> {
  const topic = matchPath("/home/:dotId/:rootRunId", pathname);
  if (topic) {
    return { screen: "dot", dot_id: topic.params.dotId!, topic_root: topic.params.rootRunId! };
  }
  const dot = matchPath("/home/:dotId", pathname);
  if (dot) return { screen: "dot", dot_id: dot.params.dotId!, topic_root: null };
  const first = pathname.split("/").filter(Boolean)[0] ?? "home";
  // The dot's editor and its graph are about one dot too.
  const other = matchPath("/:screen/:dotId", pathname);
  const dotId =
    (first === "dots" || first === "graph") && other && other.params.dotId !== "new"
      ? other.params.dotId!
      : null;
  return { screen: first, dot_id: dotId, topic_root: null };
}

/// How long the view must hold before it is sent: a burst of route changes is one call.
const VIEW_WAIT_MS = 100;

function take(qc: QueryClient, event: VoiceEvent, open: (path: string) => void): void {
  switch (event.kind) {
    case "state":
      // Listening again means whatever was wrong is not.
      voiceStore.set({
        state: event.state,
        micOpen: event.mic_open,
        ...(event.state === "listening" ? { trouble: null } : {}),
      });
      {
        const old = qc.getQueryData<VoiceStatus>(keys.voice);
        if (old) {
          takeVoiceStatus(qc, {
            ...old,
            state: event.state,
            mic_open: event.mic_open,
            ...(event.state === "listening" ? { trouble: null } : {}),
          });
        }
      }
      return;
    case "level":
      voiceStore.set({ level: { mic: event.mic, voice: event.voice } });
      return;
    case "hearing":
      voiceStore.set({ hearing: event.text, heard: false });
      return;
    case "heard":
      // What the person said in full: the reply before it is done with.
      voiceStore.set({ hearing: event.text, heard: true, said: [] });
      return;
    case "saying": {
      // The first line of a reply: what was heard gives way to it.
      const { said } = voiceStore.get();
      lastLine += 1;
      voiceStore.set({
        saying: event.text,
        said: [...said, { n: lastLine, text: event.text }].slice(-SAID_KEPT),
        ...(said.length === 0 ? { hearing: "", heard: false } : {}),
      });
      return;
    }
    case "ask":
      voiceStore.set({
        ask: { question: event.question, options: event.options, seq: event.seq },
      });
      return;
    case "card":
      voiceStore.set({ card: event.card });
      return;
    case "open":
      open(pathOf(event.target));
      return;
    case "cleared":
      voiceStore.set({ hearing: "", heard: false, saying: "", said: [], ask: null, card: null });
      return;
    case "woke":
      return;
    case "trouble":
      voiceStore.set({ trouble: event.message, troubles: voiceStore.get().troubles + 1 });
      return;
    case "download":
      voiceStore.set({ download: { done: event.done_bytes, total: event.total_bytes } });
      return;
    case "status": {
      const status = event.status;
      takeVoiceStatus(qc, status);
      voiceStore.set({
        state: status.state,
        micOpen: status.mic_open,
        trouble: status.trouble,
        ...(status.downloading ? {} : { download: null }),
      });
      // Voice turned itself on or off (a download that ended): the stored setting followed.
      const settings = qc.getQueryData<Settings>(keys.settings);
      if (settings && settings.voice_enabled !== status.enabled) invalidate(qc, keys.settings);
      return;
    }
  }
}

/// Follows Open Dot for the whole app: mounted once, in the frame. It keeps the voice status
/// in the query cache and the moment-to-moment state in `voiceStore`, opens what Open Dot
/// opens, and tells the core what the person is looking at.
export function useVoice(): void {
  const qc = useQueryClient();
  const navigate = useNavigate();
  // The latest navigate, so that a new one does not subscribe again.
  const go = useRef(navigate);
  go.current = navigate;

  useEffect(() => {
    let subscribed = true;
    // Whether an event has told the status already: it is newer than any fetch begun before.
    let heard = false;
    voiceStore.reset();
    // Listen first, then ask: an event that comes while the answer is on its way wins.
    const subscription = onVoice(
      (event) => {
        if (!subscribed) return;
        if (event.kind === "status") heard = true;
        take(qc, event, (path) => void go.current(path));
      },
      () => invalidate(qc, keys.voice),
    );
    // Through the query, so a screen that reads the status shares this read, and a status
    // event that comes first is kept (`readVoiceStatus`).
    qc.fetchQuery({ ...statusQuery(qc), staleTime: 0 })
      .then((status) => {
        if (!subscribed || heard) return;
        voiceStore.set({ state: status.state, micOpen: status.mic_open, trouble: status.trouble });
      })
      .catch(() => {});
    return () => {
      subscribed = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [qc]);

  // What the person is looking at, sent once per change.
  const scope = useCurrentScope();
  const { pathname } = useLocation();
  const part = viewOf(pathname);
  const view: View | null = scope
    ? { account: scope.account, workspace: scope.workspace, ...part }
    : null;
  const said = view && JSON.stringify(view);
  useEffect(() => {
    if (said === null) return;
    const t = setTimeout(() => {
      commands.voiceView(JSON.parse(said) as View).catch(() => {});
    }, VIEW_WAIT_MS);
    return () => clearTimeout(t);
  }, [said]);
}
