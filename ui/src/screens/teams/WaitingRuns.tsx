import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { WaitingWorkflowRun } from "@/api/types";
import { Button } from "@/components/ui/button";
import { relativeTime } from "@/lib/format";
import { keys } from "@/lib/query";

import { taskLine } from "./format";
import { TeamSection } from "./parts";
import { TestBadge, WorkflowRunBadge } from "./WorkflowRunBadge";

/// A workflow run that waits for the person (teams spec section 10.5): its task's first line,
/// its team and workflow, the step it waits at, what it waits for, and its page. The team's
/// own page leaves the team out (`team={false}`).
export function WaitingRunRow({ item, team = true }: { item: WaitingWorkflowRun; team?: boolean }) {
  const workflow = item.workflow_name ?? "A deleted workflow";
  return (
    <li
      aria-label={`${workflow} of ${item.team_name} waiting`}
      className="flex items-center gap-3 rounded-lg border p-3"
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{taskLine(item.task) || "No task."}</span>
        <span className="truncate text-xs text-muted-foreground">
          {team && (
            <>
              <Link to={`/teams/${item.team_id}`} className="text-foreground hover:underline">
                {item.team_name}
              </Link>
              {" · "}
            </>
          )}
          {workflow}
          {item.step_name && ` · at ${item.step_name}`} · started {relativeTime(item.started_at)}
        </span>
      </div>
      {item.test && <TestBadge />}
      <WorkflowRunBadge status="waiting" waitingOn={item.waiting_on} />
      <Button asChild size="sm" variant="outline">
        <Link to={`/workflow-runs/${item.id}`}>Open the run</Link>
      </Button>
    </li>
  );
}

/// The team's workflow runs that wait for the person, from the Inbox's list: nothing while
/// none waits. Live: a workflow run's event marks the Inbox stale.
export function TeamWaiting({ teamId }: { teamId: string }) {
  const inbox = useQuery({ queryKey: keys.inbox, queryFn: commands.inbox });
  const waiting = (inbox.data?.workflows ?? []).filter((item) => item.team_id === teamId);
  if (waiting.length === 0) return null;
  return (
    <TeamSection id="team-waiting" title="Waiting">
      <ul className="flex flex-col gap-2">
        {waiting.map((item) => (
          <WaitingRunRow key={item.id} item={item} team={false} />
        ))}
      </ul>
    </TeamSection>
  );
}
