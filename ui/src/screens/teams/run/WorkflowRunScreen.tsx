import { useQuery } from "@tanstack/react-query";
import { Link, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { WorkflowRun } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { keys } from "@/lib/query";

import { builderPath } from "../TeamWorkflows";
import { taskLine, TRIGGER_LABEL } from "../format";
import { TestBadge, WorkflowRunBadge } from "../WorkflowRunBadge";
import { inFlight, StopRun } from "./actions";
import { LimitPanel } from "./LimitPanel";
import { RunMeters } from "./RunMeters";
import { RunSteps, type RunView } from "./RunSteps";
import { latestStepRuns } from "./steps";

/// A workflow run's page (teams spec section 10.3, option A): live from the run's events.
export function WorkflowRunScreen() {
  const { runId = "" } = useParams();
  const run = useQuery({
    queryKey: keys.workflowRun(runId),
    queryFn: () => commands.workflowRunGet(runId),
  });
  if (run.isPending) return <Page title="Workflow run" />;
  if (run.isError) {
    return (
      <Page title="Workflow run">
        <ErrorNote error={run.error} />
      </Page>
    );
  }
  return <RunPage run={run.data} />;
}

/// The header (the workflow, the task's first line, the team and project, Test, the status,
/// Stop), the budget's meters, a wait on a limit, then the definition's steps with their step
/// runs.
function RunPage({ run }: { run: WorkflowRun }) {
  const team = useQuery({
    queryKey: keys.team(run.team_id),
    queryFn: () => commands.teamGet(run.team_id),
  });
  const workflowId = run.workflow_id;
  const workflow = useQuery({
    queryKey: keys.workflow(workflowId ?? ""),
    queryFn: () => commands.workflowGet(workflowId!),
    enabled: workflowId !== null,
  });
  const dotRuns = useQuery({
    queryKey: keys.workflowDotRuns(run.id),
    queryFn: () => commands.workflowRunDotRuns(run.id),
  });
  const names = new Map((team.data?.members ?? []).map((m) => [m.dot_id, m.dot_name]));
  const view: RunView = {
    run,
    latest: latestStepRuns(run),
    nameOf: (id) => names.get(id) ?? id,
    dotRuns: dotRuns.data ?? [],
  };
  // The workflow is gone once its id is null, or when it was deleted under the page.
  const gone = workflowId === null || workflow.isError;
  const title = gone ? "A deleted workflow" : (workflow.data?.name ?? "Workflow run");
  return (
    <div className="flex flex-col gap-4 p-6">
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          <Link to={`/teams/${run.team_id}`} className="hover:underline">
            {team.data?.name ?? "Team"}
          </Link>
          <span>· {TRIGGER_LABEL[run.trigger]}</span>
          {run.origin_run_id && (
            <Link to={`/runs/${run.origin_run_id}`} className="hover:underline">
              · Open the chat
            </Link>
          )}
        </div>
        <div className="flex items-center gap-3">
          <h1 className="min-w-0 flex-1 truncate text-lg font-semibold">
            {workflow.data && !gone ? (
              <Link to={builderPath(run.team_id, workflow.data.id)} className="hover:underline">
                {title}
              </Link>
            ) : (
              title
            )}
          </h1>
          {inFlight(run) && <StopRun run={run} />}
        </div>
        <p className="text-sm">{taskLine(run.task) || "No task."}</p>
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span>On {run.project.name}</span>
          <WorkflowRunBadge status={run.status} waitingOn={run.waiting_on} />
          {run.test && <TestBadge />}
        </div>
      </header>
      <RunMeters run={run} />
      {run.error && run.waiting_on !== "step_failed" && (
        <p role="alert" className="rounded-md border border-destructive/50 p-3 text-destructive">
          {run.error}
        </p>
      )}
      <LimitPanel run={run} stop={false} />
      <RunSteps steps={run.definition.steps} prefix="" name="Steps" view={view} />
    </div>
  );
}
