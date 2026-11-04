import { localDateTime, localWeekdayTime } from "@/lib/format";

/// "Later" presets for Snooze (snooze and done spec section 3.3), in local time. Send later
/// reuses them, with its own label for `next_week`.
export type LaterId = "hour" | "evening" | "morning" | "next_week";

export interface LaterPreset {
  id: LaterId;
  label: string;
  at: Date;
}

const DAY_MS = 86_400_000;
/// How far ahead a time may be picked: the core refuses more (spec section 3.9.10).
const REACH_MS = 365 * DAY_MS;
/// The evening and morning hours.
const EVENING = 18;
const MORNING = 9;

/// The presets at `now`, in menu order: in 1 hour; this evening at 6 PM, left out from 6 PM on
/// (a passed preset is hidden, not rolled to tomorrow); tomorrow at 9 AM; and the next Monday
/// after today at 9 AM (on a Monday, the following one).
export function laterPresets(now: Date): LaterPreset[] {
  const y = now.getFullYear();
  const m = now.getMonth();
  const d = now.getDate();
  const presets: LaterPreset[] = [
    { id: "hour", label: "In 1 hour", at: new Date(now.getTime() + 3_600_000) },
  ];
  if (now.getHours() < EVENING) {
    presets.push({ id: "evening", label: "This evening (6 PM)", at: new Date(y, m, d, EVENING) });
  }
  presets.push({
    id: "morning",
    label: "Tomorrow morning (9 AM)",
    at: new Date(y, m, d + 1, MORNING),
  });
  // Monday is day 1: the days to the next Monday, 7 on a Monday itself.
  const toMonday = (8 - now.getDay()) % 7 || 7;
  presets.push({
    id: "next_week",
    label: "Next week (Monday 9 AM)",
    at: new Date(y, m, d + toMonday, MORNING),
  });
  return presets;
}

/// A time the core takes: after `now`, and at most 365 days ahead.
export function isLater(at: Date, now: Date): boolean {
  const ahead = at.getTime() - now.getTime();
  return ahead > 0 && ahead <= REACH_MS;
}

/// A `datetime-local` value ("2026-11-05T09:00") as a local time; null for none or a value that
/// is not one.
export function fromPicker(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(value);
  if (!match) return null;
  const [, y, mo, d, h, mi, s] = match.map(Number);
  const at = new Date(y, mo - 1, d, h, mi, Number.isNaN(s) ? 0 : s);
  return Number.isNaN(at.getTime()) ? null : at;
}

/// A local time as a `datetime-local` value, to the minute.
export function toPicker(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

/// When a snoozed item comes back: "Back 6:00 PM" today, "Back Mon 9:00 AM" within 6 days,
/// else "Back" and the local date and time.
export function backText(iso: string, now: number): string {
  const t = Date.parse(iso);
  const when = t - now <= 6 * DAY_MS ? localWeekdayTime(iso, now) : localDateTime(iso);
  return `Back ${when}`;
}
