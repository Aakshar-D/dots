import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { LimitReading, LimitsState } from "@/api/types";
import { relativeTime } from "@/lib/format";
import { keys } from "@/lib/query";
import { pct, resetTime } from "@/lib/usage-format";

/// A reading older than this shows its age (usage and limits spec section 5).
const STALE_MS = 10 * 60_000;

const part = (label: string, r: LimitReading | null) =>
  r ? `${label} ${pct(r.utilization)}%, resets ${resetTime(r.resets_at)}` : `${label} unknown`;

/// "Claude usage: 5-hour 42%, resets 15:10; weekly 55%, resets Thu 09:00; holding 0 runs"
export function limitsName(limits: LimitsState): string {
  return `Claude usage: ${part("5-hour", limits.five_hour)}; ${part("weekly", limits.weekly)}; holding ${limits.held_run_ids.length} runs`;
}

/// The name, and the age of the oldest reading once it is older than 10 minutes.
function limitsTitle(limits: LimitsState, now: number): string {
  const read = [limits.five_hour, limits.weekly]
    .filter((r): r is LimitReading => r !== null)
    .map((r) => r.read_at)
    .sort()[0];
  const stale = read !== undefined && now - Date.parse(read) > STALE_MS;
  return stale ? `${limitsName(limits)}; read ${relativeTime(read, now)}` : limitsName(limits);
}

function Bar(props: { window: string; reading: LimitReading | null; holding: boolean }) {
  const level = props.reading ? pct(props.reading.utilization) : null;
  return (
    <span
      data-window={props.window}
      data-level={level ?? "unknown"}
      data-holding={props.holding || undefined}
      className={cn(
        "relative h-7 w-1.5 overflow-hidden rounded-full",
        level === null ? "border border-dashed border-muted-foreground" : "bg-muted",
      )}
    >
      {level !== null && (
        <span
          className={cn(
            "absolute inset-x-0 bottom-0 rounded-full",
            props.holding ? "bg-needs" : "bg-primary",
          )}
          style={{ height: `${level}%` }}
        />
      )}
    </span>
  );
}

/// The rail's Claude usage (usage and limits spec section 5): the 5-hour and weekly bars of the
/// whole account, amber while background runs are held; a link to the Usage page.
export function LimitsMeter() {
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  if (!limits) return null;
  const name = limitsName(limits);
  return (
    <Link
      to="/usage"
      aria-label={name}
      title={limitsTitle(limits, Date.now())}
      className="mt-auto flex size-9 items-end justify-center gap-1 rounded-lg pb-1 hover:bg-sidebar-accent"
    >
      <Bar window="five_hour" reading={limits.five_hour} holding={limits.holding} />
      <Bar window="seven_day" reading={limits.weekly} holding={limits.holding} />
    </Link>
  );
}
