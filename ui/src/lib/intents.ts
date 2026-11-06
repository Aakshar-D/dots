import type { Intent } from "@/api/types";

/// Where an open goal shows on the Intents screen, by how soon it matters.
export type GroupKey = "now" | "overdue" | "today" | "week" | "later" | "none";

/// The groups in the order they are shown, with their headings.
export const GROUPS: { key: GroupKey; label: string }[] = [
  { key: "now", label: "Now" },
  { key: "overdue", label: "Overdue" },
  { key: "today", label: "Today" },
  { key: "week", label: "This week" },
  { key: "later", label: "Later" },
  { key: "none", label: "No date" },
];

/// "This week" runs from tomorrow to this many days from today.
export const WEEK_DAYS = 7;

const pad = (n: number) => String(n).padStart(2, "0");

/// The calendar date of `d` in local time, `YYYY-MM-DD`.
export function localDate(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/// The calendar date `days` after `date` (`YYYY-MM-DD`).
export function addDays(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return localDate(new Date(y, m - 1, d + days));
}

/// The group of an open goal on `today` (`YYYY-MM-DD`, local): an urgent goal is Now whatever
/// its date. Dates are `YYYY-MM-DD`, so they compare as text.
export function groupOf(intent: Intent, today: string): GroupKey {
  if (intent.urgent) return "now";
  const due = intent.due_date;
  if (due === null) return "none";
  if (due < today) return "overdue";
  if (due === today) return "today";
  if (due <= addDays(today, WEEK_DAYS)) return "week";
  return "later";
}

export type IntentGroup = { key: GroupKey; label: string; intents: Intent[] };

/// The open goals in their groups, in order, empty groups left out; each group soonest date
/// first, then oldest. The done goals apart, the latest done first.
export function groupIntents(
  intents: Intent[],
  today: string = localDate(new Date()),
): { groups: IntentGroup[]; done: Intent[] } {
  const open = intents.filter((i) => i.done_at === null);
  const byDate = (a: Intent, b: Intent) =>
    (a.due_date ?? "").localeCompare(b.due_date ?? "") ||
    a.created_at.localeCompare(b.created_at) ||
    a.id.localeCompare(b.id);
  const groups = GROUPS.map(({ key, label }) => ({
    key,
    label,
    intents: open.filter((i) => groupOf(i, today) === key).sort(byDate),
  })).filter((g) => g.intents.length > 0);
  const done = intents
    .filter((i) => i.done_at !== null)
    .sort((a, b) => (b.done_at ?? "").localeCompare(a.done_at ?? ""));
  return { groups, done };
}
