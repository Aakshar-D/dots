// A dot's runs as topics, and which of them make its conversation on the home (chat-first home
// spec, section 2).
import type { Run } from "@/api/types";
import { localTime } from "@/lib/format";
import { watcherTitle } from "@/lib/watchers";

/// One line of runs: a root run and the runs that continued it (`root_run_id`).
export type Topic = {
  /// The root's id: the topic's name in the address.
  rootId: string;
  /// The topic's oldest run known here: its root, or the oldest run of it that was loaded when
  /// the root is older than what was loaded.
  root: Run;
  /// Its runs, oldest first.
  runs: Run[];
  /// "chat" when the root is a chat run; "run" when a schedule, a webhook, Run now or a
  /// follow-up started it.
  kind: "chat" | "run";
};

/// How many run topics make the conversation of a dot that has no chat topic.
export const RUN_TOPICS = 20;
/// How many older topics History lists at most.
export const HISTORY = 50;

/// Whether the run has not ended: it waits to start, runs, or waits for an approval. The one
/// definition for every screen (the core's is `RunStatus::is_terminal`).
export function isActive(run: Run): boolean {
  return run.status === "queued" || run.status === "running" || run.status === "awaiting_approval";
}

const byQueued = (a: Run, b: Run) => a.queued_at.localeCompare(b.queued_at);

/// The topic of one line of runs, given in any order and none missing between them.
export function topicOf(runs: Run[]): Topic {
  const sorted = [...runs].sort(byQueued);
  const root = sorted.find((run) => run.id === run.root_run_id) ?? sorted[0];
  return {
    rootId: sorted[0].root_run_id,
    root,
    runs: sorted,
    kind: root.trigger === "chat" ? "chat" : "run",
  };
}

/// A dot's runs as topics, ordered by the time their roots were queued, oldest first.
export function topicsOf(runs: Run[]): Topic[] {
  const lines = new Map<string, Run[]>();
  for (const run of runs) {
    const line = lines.get(run.root_run_id) ?? [];
    line.push(run);
    lines.set(run.root_run_id, line);
  }
  return [...lines.values()].map(topicOf).sort((a, b) => byQueued(a.root, b.root));
}

/// The newest chat topic, by the time its root was queued.
export function currentTopic(topics: Topic[]): Topic | undefined {
  return topics.filter((topic) => topic.kind === "chat").at(-1);
}

/// The topics of the conversation, oldest first: the current topic and the last `RUN_TOPICS`
/// run topics whose roots were queued after its root; with no chat topic, the last
/// `RUN_TOPICS` run topics. The rest are in History.
export function conversation(topics: Topic[]): Topic[] {
  const current = currentTopic(topics);
  if (current === undefined) return topics.slice(-RUN_TOPICS);
  const after = topics.filter(
    (topic) =>
      topic.kind === "run" && topic.root.queued_at.localeCompare(current.root.queued_at) > 0,
  );
  return [current, ...after.slice(-RUN_TOPICS)];
}

/// The topics History lists: those not in the conversation, newest first, `HISTORY` at most.
export function olderTopics(topics: Topic[], shown: Topic[]): Topic[] {
  const inConversation = new Set(shown.map((topic) => topic.rootId));
  return topics
    .filter((topic) => !inConversation.has(topic.rootId))
    .reverse()
    .slice(0, HISTORY);
}

/// The newest run of a topic: what a reply continues.
export function lastRun(topic: Topic): Run {
  return topic.runs.at(-1)!;
}

/// The topic a message goes to by default: the current chat topic of those shown, whatever ran
/// after it; none (a new chat topic) when only run topics are shown. A run topic gets a message
/// only through its Reply, so the target never moves while the person types.
export function defaultTarget(shown: Topic[]): Topic | undefined {
  return currentTopic(shown);
}

const TRIGGER: Record<Run["trigger"], string> = {
  schedule: "Scheduled",
  webhook: "Webhook",
  manual: "Run now",
  chat: "Chat",
  resume: "Continued",
  follow_up: "Follow-up",
  app: "App",
  workflow: "Workflow",
  watcher: "Watcher",
};

/// What started a run, in a word or two: "Scheduled", "Webhook", "Run now".
export function triggerLabel(run: Run): string {
  return TRIGGER[run.trigger];
}

/// The first line of a string, trimmed and cut to `max` characters; "" for none.
export function oneLine(text: string | null | undefined, max = 80): string {
  const line = (text ?? "").trim().split("\n")[0].trim();
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

/// What the person wrote to start a chat run: an answer as they typed it, else the message.
export function chatText(run: Run): string | null {
  if (typeof run.payload !== "object" || run.payload === null) return null;
  const { answer, message } = run.payload as Record<string, unknown>;
  if (typeof answer === "string") return answer;
  return typeof message === "string" ? message : null;
}

/// A topic's name: the first line of its root's message for a chat topic, else its trigger and
/// the time it was queued ("Scheduled · 08:00").
export function topicTitle(topic: Topic): string {
  if (topic.kind === "chat") return oneLine(chatText(topic.root), 60) || "Chat";
  // A watcher's run says what woke the dot (watchers spec section 8).
  if (topic.root.trigger === "watcher") {
    const title = watcherTitle(topic.root);
    if (title) return title;
  }
  return `${triggerLabel(topic.root)} · ${localTime(topic.root.queued_at)}`;
}
