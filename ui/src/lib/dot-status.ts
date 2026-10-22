// What the home's column says about each dot (chat-first home spec, section 3).
import type { DotOverview, FollowUp, Message } from "@/api/types";
import { localTime, relativeTime } from "@/lib/format";
import { chatText, oneLine } from "@/lib/home-topics";
import { isOpenQuestion } from "@/lib/messages";

/// A dot's status mark, in the order the first match wins.
export type DotStatus = "waiting" | "running" | "attention" | "off" | "idle";

export const STATUS_LABEL: Record<DotStatus, string> = {
  waiting: "waiting for you",
  running: "running",
  attention: "needs attention",
  off: "off",
  idle: "idle",
};

/// Waiting (a run waits for an approval, or the dot asked a question still open), running (a
/// run is queued or running), attention (its schedule needs attention), off (disabled), else
/// idle. `messages`: any messages; only the dot's open questions count.
export function dotStatus(row: DotOverview, messages: Message[]): DotStatus {
  const active = row.active_run;
  if (
    active?.status === "awaiting_approval" ||
    messages.some((m) => m.dot_id === row.dot.id && isOpenQuestion(m))
  ) {
    return "waiting";
  }
  if (active?.status === "queued" || active?.status === "running") return "running";
  if (row.schedule_state?.attention) return "attention";
  if (!row.dot.enabled) return "off";
  return "idle";
}

/// When the dot last did something: the later of its active and last runs' queue times.
function activity(row: DotOverview): string | null {
  const times = [row.active_run?.queued_at, row.last_run?.queued_at].filter(
    (t): t is string => typeof t === "string",
  );
  return times.length === 0 ? null : times.sort().at(-1)!;
}

/// The column's order: newest activity first; the dots that never ran after them, by name.
export function sortDots(rows: DotOverview[]): DotOverview[] {
  return [...rows].sort((a, b) => {
    const [x, y] = [activity(a), activity(b)];
    if (x !== null && y !== null) return y.localeCompare(x);
    if (x !== null) return -1;
    if (y !== null) return 1;
    return a.dot.name.localeCompare(b.dot.name);
  });
}

/// The row's last line: the last run's summary, else its error, else its chat message, on one
/// line; "" without a last run.
export function lastLine(row: DotOverview): string {
  const run = row.last_run;
  if (run === null) return "";
  return oneLine(run.summary) || oneLine(run.error) || oneLine(chatText(run));
}

/// "next 02:00" for the next scheduled fire, else when the dot's next follow-up fires ("Next
/// follow-up in 3 h"), else "". `followUps`: the pending ones of every dot, soonest first.
export function nextLine(row: DotOverview, followUps: FollowUp[]): string {
  const fire = row.schedule_state?.next_fire;
  if (fire) return `next ${localTime(fire)}`;
  const followUp = followUps.find((f) => f.dot_id === row.dot.id);
  return followUp ? `Next follow-up ${relativeTime(followUp.due_at)}` : "";
}
