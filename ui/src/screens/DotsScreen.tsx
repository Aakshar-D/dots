import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, Plus } from "lucide-react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
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
import { localDateTime, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";

/// When the dot runs next on its schedule, or why it will not.
export function nextRun(row: DotOverview, schedulesPaused: boolean): string {
  if (!row.dot.schedule) return "No schedule";
  if (!row.dot.enabled) return "Disabled";
  if (schedulesPaused) return "Paused";
  return localDateTime(row.next_fire) || "Not scheduled";
}

export function DotsScreen() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const overview = useQuery({ queryKey: keys.overview, queryFn: commands.dotOverview });
  const status = useQuery({ queryKey: keys.status, queryFn: commands.status }).data;
  const runNow = useMutation({
    mutationFn: commands.runNow,
    onSuccess: (run) => void navigate(`/runs/${run.id}`),
    onError: notifyError,
  });
  const setEnabled = useMutation({
    mutationFn: (v: { id: string; enabled: boolean }) => commands.setDotEnabled(v.id, v.enabled),
    // The editor fills its form from the cached dot when it opens, so that copy must never be
    // older than this change. It is dropped before the command goes out, and the command's
    // answer is the new copy; without an answer the editor loads the dot itself.
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
            return (
              <TableRow key={dot.id}>
                <TableCell>
                  <Link to={`/dots/${dot.id}`} className="font-medium hover:underline">
                    {dot.name}
                  </Link>
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
                <TableCell>{nextRun(row, status?.schedules_paused ?? false)}</TableCell>
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
                    disabled={setEnabled.isPending}
                    onCheckedChange={(enabled) => setEnabled.mutate({ id: dot.id, enabled })}
                  />
                </TableCell>
                <TableCell className="text-right">
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={runNow.isPending}
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
