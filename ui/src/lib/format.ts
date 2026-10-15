const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
const timeOfDay = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });

/// An RFC 3339 timestamp in the user's locale and time zone. "" for none; the text itself
/// when it is not a timestamp.
export function localDateTime(iso: string | null | undefined): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : dateTime.format(t);
}

/// The time of day of an RFC 3339 timestamp, in the user's locale and time zone, for a time
/// that is close enough not to need its date. "" for none; the text itself when it is not a
/// timestamp.
export function localTime(iso: string | null | undefined): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  return Number.isNaN(t) ? iso : timeOfDay.format(t);
}

/// "45s", "3m 05s", "2h 07m". Without an end the run is still going: measured up to `now`.
export function duration(
  startIso: string | null,
  endIso: string | null,
  now: number = Date.now(),
): string {
  if (!startIso) return "";
  const ms = (endIso ? Date.parse(endIso) : now) - Date.parse(startIso);
  if (!Number.isFinite(ms) || ms < 0) return "";
  const s = Math.floor(ms / 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${pad(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${pad(Math.floor((s % 3600) / 60))}m`;
}

/// "just now", "5 min ago", "3 h ago", "2 d ago", and "in 5 min" for a time ahead.
export function relativeTime(iso: string | null | undefined, now: number = Date.now()): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const ahead = t > now;
  const ms = Math.abs(t - now);
  if (ms < 45_000) return ahead ? "now" : "just now";
  const [n, unit] =
    ms < 3_600_000
      ? [Math.round(ms / 60_000), "min"]
      : ms < 86_400_000
        ? [Math.round(ms / 3_600_000), "h"]
        : [Math.round(ms / 86_400_000), "d"];
  return ahead ? `in ${n} ${unit}` : `${n} ${unit} ago`;
}
