import type { Run } from "@/api/types";
import { localTime } from "@/lib/format";

/// "950", "12k", "182k", "2.1M".
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
}

/// "45s", "12m 04s", "3h 12m".
export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const pad = (n: number) => String(n).padStart(2, "0");
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${pad(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${pad(Math.floor((s % 3600) / 60))}m`;
}

/// "$1.32"; "<$0.01" for a cost that would round to nothing.
export function formatCost(usd: number): string {
  return usd > 0 && usd < 0.005 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

/// The share used in whole percent, at most 100 (the CLI may report more than the window).
export function pct(utilization: number): number {
  return Math.min(100, Math.round(utilization * 100));
}

/// A run's time from start to end in seconds; null until it has ended.
export function runSeconds(run: Pick<Run, "started_at" | "ended_at">): number | null {
  if (!run.started_at || !run.ended_at) return null;
  const ms = Date.parse(run.ended_at) - Date.parse(run.started_at);
  return Number.isFinite(ms) && ms >= 0 ? Math.round(ms / 1000) : null;
}

const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });

/// When a window resets: the time of day when it is today, else with the weekday ("Thu 09:00").
export function resetTime(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const sameDay = new Date(t).toDateString() === new Date(now).toDateString();
  return sameDay ? localTime(iso) : `${weekday.format(t)} ${localTime(iso)}`;
}
