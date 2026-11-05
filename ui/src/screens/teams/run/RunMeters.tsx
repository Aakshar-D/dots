import { cn } from "cn";

import type { WorkflowRun } from "@/api/types";
import { formatTokens } from "@/lib/usage-format";

import { limits } from "./steps";

/// The run's budget across the top (section 10.3): runs, tokens, minutes and send-backs, each
/// "spent / limit" over a bar, amber once reached. Cache reads are shown beside the tokens: they
/// are not counted against the limit.
export function RunMeters({ run }: { run: WorkflowRun }) {
  return (
    <ul aria-label="Budget" className="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {limits(run).map((l) => {
        const cache = l.key === "max_tokens" && run.spent_cache_read > 0;
        const name =
          `${l.label}: ${l.text.replace(" / ", " of ")}` +
          (cache ? `, and ${formatTokens(run.spent_cache_read)} read from the cache` : "") +
          (l.reached ? ", reached" : "");
        const share = l.limit > 0 ? Math.min(100, Math.round((l.spent / l.limit) * 100)) : 100;
        return (
          <li
            key={l.key}
            aria-label={name}
            data-reached={l.reached || undefined}
            className="flex flex-col gap-1 rounded-lg border bg-card p-2"
          >
            <span className="flex items-baseline gap-2 text-xs text-muted-foreground">
              {l.label}
              {cache && <span>· {formatTokens(run.spent_cache_read)} cache reads</span>}
            </span>
            <span className={cn("text-sm font-semibold", l.reached && "text-needs")}>
              {l.text}
            </span>
            <span aria-hidden className="h-1.5 overflow-hidden rounded-full bg-muted">
              <span
                className={cn("block h-full rounded-full", l.reached ? "bg-needs" : "bg-primary")}
                style={{ width: `${share}%` }}
              />
            </span>
          </li>
        );
      })}
    </ul>
  );
}
