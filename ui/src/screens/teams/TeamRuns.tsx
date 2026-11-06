import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { Workflow } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { localDateTime } from "@/lib/format";
import { keys } from "@/lib/query";

import { formatSpend, taskLine } from "./format";
import { TeamSection } from "./parts";
import { TestBadge, WorkflowRunBadge } from "./WorkflowRunBadge";

/// How many of the team's newest workflow runs its page lists.
export const RECENT_RUNS = 20;

/// The team's newest workflow runs (teams spec section 10.1): status, task, workflow, project
/// and spend, each opening its run page. Live: a workflow run's event marks the list stale.
export function TeamRuns({ teamId, workflows }: { teamId: string; workflows: Workflow[] }) {
  const filter = { team_id: teamId, limit: RECENT_RUNS };
  const runs = useQuery({
    queryKey: keys.workflowRuns(filter),
    queryFn: () => commands.workflowRunsList(filter),
  });
  // Blank while the workflows load.
  const workflowName = (id: string | null) =>
    id === null ? "A deleted workflow" : (workflows.find((w) => w.id === id)?.name ?? "");
  return (
    <TeamSection id="team-runs" title="Recent runs">
      {runs.isError && <ErrorNote error={runs.error} />}
      {runs.data?.length === 0 && <p className="text-muted-foreground">No runs yet.</p>}
      {(runs.data ?? []).length > 0 && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Status</TableHead>
              <TableHead>Task</TableHead>
              <TableHead>Workflow</TableHead>
              <TableHead>Project</TableHead>
              <TableHead>Spent</TableHead>
              <TableHead>Started</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {runs.data!.map((run) => (
              <TableRow key={run.id}>
                <TableCell>
                  <span className="flex items-center gap-1">
                    <WorkflowRunBadge status={run.status} waitingOn={run.waiting_on} />
                    {run.test && <TestBadge />}
                  </span>
                </TableCell>
                <TableCell className="max-w-sm truncate">
                  <Link to={`/workflow-runs/${run.id}`} className="hover:underline">
                    {taskLine(run.task)}
                  </Link>
                </TableCell>
                <TableCell>{workflowName(run.workflow_id)}</TableCell>
                <TableCell>{run.project.name}</TableCell>
                <TableCell className="tabular-nums">{formatSpend(run)}</TableCell>
                <TableCell>{localDateTime(run.started_at)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </TeamSection>
  );
}
