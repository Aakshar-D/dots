import type { Run } from "@/api/types";

export const WAIT_REASON_TEXT: Record<NonNullable<Run["wait_reason"]>, string> = {
  busy: "other runs are busy",
  usage_hold: "held for the usage limit",
  updating: "Sage is updating",
  dot_busy: "this sol is busy with another run",
  queued: "next in line",
  spend_cap: "a spending cap is reached",
};

const MINUTE_MS = 60_000;
/// A message waits this many whole minutes before the line says so.
const WAIT_SHOWN_MIN = 5;

export type DeliveryLineInfo = {
  kind: "waiting" | "not_delivered" | "dismissed";
  text: string;
};

/// What to say under a message about whether it got there, or nothing (delivery tracking spec
/// section 6). `now` is milliseconds since the epoch.
export function deliveryLine(run: Run, now: number): DeliveryLineInfo | null {
  if (run.delivery === "not_delivered") {
    if (run.dismissed_at) return { kind: "dismissed", text: "Not delivered" };
    const reason = run.undelivered_reason?.trim();
    return { kind: "not_delivered", text: reason ? `Not delivered: ${reason}` : "Not delivered" };
  }
  if (run.delivery === "waiting" && run.status === "queued" && run.wait_reason) {
    const minutes = Math.floor((now - Date.parse(run.queued_at)) / MINUTE_MS);
    if (minutes >= WAIT_SHOWN_MIN) {
      return {
        kind: "waiting",
        text: `Waiting ${minutes} min · ${run.wait_detail ?? WAIT_REASON_TEXT[run.wait_reason]}`,
      };
    }
  }
  return null;
}
