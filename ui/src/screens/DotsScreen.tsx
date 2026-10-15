import { useMutation, useMutationState, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, Plus } from "lucide-react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import { LIMITS, type DotOverview } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { localDateTime, localTime, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidateDots, keys, setDotEnabledMutation } from "@/lib/query";

/// The key of this screen's "Run now", so that the ones on their way can be found.
const RUN_NOW = ["run-now"] as const;

/// When the dot runs next on its schedule, or why it will not. An ended schedule says how it
/// ended, paused or not: a schedule that completed without firing once had its only slot
/// missed and skipped.
export function nextRun(row: DotOverview, schedulesPaused: boolean): string {
  if (!row.dot.schedule) return "No schedule";
  if (!row.dot.enabled) return "Disabled";
  const state = row.schedule_state;
  if (state?.status === "completed") {
    return state.fired_count === 0 ? "Completed (missed)" : "Completed";
  }
  if (state?.status === "expired") return "Expired";
  if (schedulesPaused) return "Paused";
  return localDateTime(state?.next_fire) || "Not scheduled";
}

/// When the retry of the dot's last occurrence starts, and which attempt it is; null when no
/// retry waits, or when none can start (the dot is off or schedules are paused).
export function retrying(row: DotOverview, schedulesPaused: boolean): string | null {
  const state = row.schedule_state;
  if (!row.dot.enabled || schedulesPaused || !state?.retry_at || state.retry_attempt === null) {
    return null;
  }
  const of = LIMITS.max_schedule_attempts;
  return `Retrying at ${localTime(state.retry_at)} (attempt ${state.retry_attempt} of ${of})`;
}

export function DotsScreen() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const overview = useQuery({ queryKey: keys.overview, queryFn: commands.dotOverview });
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  const runNow = useMutation({
    mutationKey: RUN_NOW,
    mutationFn: commands.runNow,
    onSuccess: (run) => void navigate(`/runs/${run.id}`),
    onError: notifyError,
  });
  // A row's switch and its Run button are held while that dot's own command is on its way,
  // and no other row's. Each command is its own mutation, so several can be pending at once,
  // which `runNow.isPending` and `setEnabled.isPending` cannot tell apart.
  const starting = useMutationState({
    filters: { mutationKey: RUN_NOW, status: "pending" },
    select: (mutation) => mutation.state.variables as string,
  });
  const switching = useMutationState({
    filters: { mutationKey: setDotEnabledMutation.mutationKey, status: "pending" },
    select: (mutation) => (mutation.state.variables as { id: string }).id,
  });
  const setEnabled = useMutation({
    ...setDotEnabledMutation,
    // The editor fills its form from the cached dot when it opens, so that copy must never be
    // older than this change. It is dropped before the command goes out, and the command's
    // answer is the new copy; without an answer the editor loads the dot itself. An editor
    // opened while the command is on its way waits for it (`useSwitchingDot`).
    onMutate: (v) => qc.removeQueries({ queryKey: keys.dot(v.id) }),
    onSuccess: (dot) => void qc.setQueryData(keys.dot(dot.id), dot),
    onSettled: () => invalidateDots(qc),
    onError: notifyError,
  });

  const newDot = (
    <Button asChild size="sm">
      <Link to="/dots/new">
        <Plus aria-hidden /> New dot
      </Link>
    </Button>
  );
  if (overview.isPending) return <Page title="Dots" actions={newDot} />;
  if (overview.isError) {
    return (
      <Page title="Dots" actions={newDot}>
        <ErrorNote error={overview.error} />
      </Page>
    );
  }
  if (overview.data.length === 0) {
    return (
      <Page title="Dots" actions={newDot}>
        <p className="text-muted-foreground">
          No dots yet. A dot is an agent with instructions, a folder to work in, and a schedule
          or a webhook that starts it.
        </p>
      </Page>
    );
  }
  return (
    <Page title="Dots" actions={newDot}>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Engine</TableHead>
            <TableHead>State</TableHead>
            <TableHead>Next run</TableHead>
            <TableHead>Last result</TableHead>
            <TableHead>Enabled</TableHead>
            <TableHead />
          </TableRow>
        </TableHeader>
        <TableBody>
          {overview.data.map((row) => {
            const { dot, active_run: active, last_run: last } = row;
            const paused = status?.schedules_paused ?? false;
            const attention = row.schedule_state?.attention;
            const retry = retrying(row, paused);
            return (
              <TableRow key={dot.id}>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <Link to={`/dots/${dot.id}`} className="font-medium hover:underline">
                      {dot.name}
                    </Link>
                    {/* The Inbox lists the run that failed, with Run now and Dismiss. */}
                    {attention && (
                      <Badge variant="destructive" title={attention.error ?? undefined}>
                        Needs attention
                      </Badge>
                    )}
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">
                  {dot.engine} · {dot.model}
                </TableCell>
                <TableCell>
                  {active ? (
                    <Link to={`/runs/${active.id}`} aria-label={`Open the active run of ${dot.name}`}>
                      <StatusBadge status={active.status} />
                    </Link>
                  ) : (
                    <span className="text-muted-foreground">Idle</span>
                  )}
                </TableCell>
                <TableCell>
                  <div className="flex flex-col">
                    <span>{nextRun(row, paused)}</span>
                    {retry && <span className="text-xs text-muted-foreground">{retry}</span>}
                  </div>
                </TableCell>
                <TableCell>
                  {last ? (
                    <Link
                      to={`/runs/${last.id}`}
                      aria-label={`Open the last run of ${dot.name}`}
                      className="inline-flex items-center gap-2"
                    >
                      <StatusBadge status={last.status} />
                      <span className="text-muted-foreground">
                        {relativeTime(last.ended_at ?? last.queued_at)}
                      </span>
                    </Link>
                  ) : (
                    <span className="text-muted-foreground">Never ran</span>
                  )}
                </TableCell>
                <TableCell>
                  <Switch
                    aria-label={`${dot.name} enabled`}
                    checked={dot.enabled}
                    disabled={switching.includes(dot.id)}
                    onCheckedChange={(enabled) => setEnabled.mutate({ id: dot.id, enabled })}
                  />
                </TableCell>
                <TableCell className="text-right">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={starting.includes(dot.id)}
                    onClick={() => runNow.mutate(dot.id)}
                    aria-label={`Run ${dot.name} now`}
                  >
                    <Play aria-hidden /> Run now
                  </Button>
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </Page>
  );
}
