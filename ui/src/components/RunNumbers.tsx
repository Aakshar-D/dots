import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { LimitsState, Run } from "@/api/types";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import {
  formatCost,
  formatDuration,
  formatTokens,
  pct,
  resetTime,
  runSeconds,
} from "@/lib/usage-format";

const API_EQ =
  "Your subscription does not bill per run: this is what the run would cost on the API.";

/// "12m 04s · 182k tokens (170k in, 12k out) · $1.32 API-eq." (usage and limits spec section 5).
/// No time while the run goes on, no cost when the engine gave none.
export function RunNumbers(props: { run: Run }) {
  const { run } = props;
  const secs = runSeconds(run);
  const tokens = `${formatTokens(run.tokens_in + run.tokens_out)} tokens (${formatTokens(run.tokens_in)} in, ${formatTokens(run.tokens_out)} out)`;
  return (
    <span className="text-xs text-muted-foreground">
      {secs !== null && `${formatDuration(secs)} · `}
      {tokens}
      {run.cost_usd !== null && (
        <>
          {` · ${formatCost(run.cost_usd)} `}
          <abbr title={API_EQ} className="no-underline">
            API-eq.
          </abbr>
        </>
      )}
    </span>
  );
}

/// Why a held run waits: the window over its share and when it resets, Claude's limit, or its
/// turn after the reset.
function heldLine(limits: LimitsState): string {
  const over = [
    { label: "5-hour", reading: limits.five_hour, share: limits.shares.five_hour },
    { label: "weekly", reading: limits.weekly, share: limits.shares.weekly },
  ].find((w) => w.reading !== null && pct(w.reading.utilization) >= w.share);
  if (limits.holding && over?.reading && limits.hold_until) {
    return `Held: Claude ${over.label} at ${pct(over.reading.utilization)}%, starts after ${resetTime(limits.hold_until)}`;
  }
  if (limits.rejected_until) {
    return `Held: Claude's limit is reached, starts after ${resetTime(limits.rejected_until)}`;
  }
  return "Held: waiting for the first run after the reset";
}

/// A queued run held for Claude usage: why, and **Run anyway** (usage and limits spec section 4).
/// Nothing for a run that is not held.
export function HeldNote(props: { run: Run }) {
  const qc = useQueryClient();
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  const release = useMutation({
    mutationFn: () => commands.runReleaseHold(props.run.id),
    onError: notifyError,
    onSettled: () => {
      for (const queryKey of [keys.limits, keys.inbox]) invalidate(qc, queryKey);
    },
  });
  if (props.run.status !== "queued" || !limits?.held_run_ids.includes(props.run.id)) return null;
  return (
    <div className="flex flex-wrap items-center gap-2 text-sm">
      <span className="text-needs">{heldLine(limits)}</span>
      <Button
        size="xs"
        variant="outline"
        disabled={release.isPending}
        onClick={onSingleClick(() => release.mutate())}
      >
        Run anyway
      </Button>
    </div>
  );
}
