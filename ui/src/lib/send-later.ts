// Send later (send later spec section 6): the texts of a scheduled message's card and of the
// line under a message that was sent later. The times it can be scheduled for are the presets
// of `@/lib/later`.
import { z } from "zod";

import type { Run, ScheduledWait } from "@/api/types";
import { localDayTime } from "@/lib/format";

const MINUTE_MS = 60_000;

/// "Sends in 2h 14m" until `dueAt`, else "Sending…". Whole minutes, rounded up, as days, hours
/// and minutes, of which the two largest that are not zero are shown.
export function sendsIn(dueAt: string, now: number): string {
  const ms = Date.parse(dueAt) - now;
  if (!(ms > 0)) return "Sending…";
  const minutes = Math.ceil(ms / MINUTE_MS);
  const units: [number, string][] = [
    [Math.floor(minutes / 1440), "d"],
    [Math.floor((minutes % 1440) / 60), "h"],
    [minutes % 60, "m"],
  ];
  const shown = units
    .filter(([n]) => n > 0)
    .slice(0, 2)
    .map(([n, unit]) => `${n}${unit}`);
  return `Sends in ${shown.join(" ")}`;
}

/// Why a due message has not gone yet.
export function waitingText(waiting: ScheduledWait, dotName: string): string {
  const texts: Record<ScheduledWait, string> = {
    busy: `Due · waiting for ${dotName} to finish this topic`,
    approval: "Due · waiting for your approval in this topic",
    usage_limit: "Due · waiting for the usage limit to reset",
  };
  return texts[waiting];
}

/// The note the core writes into the payload of a chat run a scheduled message became.
export const SentLaterNote = z.object({
  id: z.string(),
  due_at: z.string(),
  late: z.boolean(),
  new_topic: z.boolean(),
});

/// The lines under the person's bubble of a chat run that a scheduled message became: whether
/// it went late or as scheduled, and that it started a new topic when it did. None for any
/// other run.
export function sentLaterLines(run: Run): string[] {
  if (run.trigger !== "chat" || typeof run.payload !== "object" || run.payload === null) {
    return [];
  }
  const note = SentLaterNote.safeParse((run.payload as Record<string, unknown>).send_later);
  if (!note.success) return [];
  const lines = [
    note.data.late
      ? `Sent late (was due ${localDayTime(note.data.due_at, Date.parse(run.queued_at))})`
      : "Sent as scheduled",
  ];
  if (note.data.new_topic) {
    lines.push("Started a new topic: the one it was written in can't be continued.");
  }
  return lines;
}
