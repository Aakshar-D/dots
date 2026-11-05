import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Team, Workflow } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { ItemRow, plural } from "@/screens/library/parts";

import { newWorkflowSpec, TRIGGER_LABEL, uniqueName } from "./format";
import { TeamSection } from "./parts";
import { RunNowDialog } from "./RunNowDialog";
import { TestBadge, WorkflowRunBadge } from "./WorkflowRunBadge";

/// The builder's route for a workflow of a team.
export const builderPath = (teamId: string, workflowId: string) =>
  `/teams/${teamId}/workflows/${workflowId}`;

/// The team's workflows (teams spec section 10.1): each with its trigger, its switch, its last
/// run, why it cannot start (its `problem`) and "Run now"; and "New workflow", which makes one
/// and opens it in the builder. `workflows` is undefined while they load.
export function TeamWorkflows({ team, workflows }: { team: Team; workflows?: Workflow[] }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [running, setRunning] = useState<Workflow | null>(null);
  const first = team.members[0];
  const create = useMutation({
    mutationFn: (member: string) => {
      const name = uniqueName("New workflow", (workflows ?? []).map((w) => w.name));
      return commands.workflowCreate(team.id, newWorkflowSpec(name, member));
    },
    onSuccess: (workflow) => {
      invalidate(qc, keys.workflows(team.id));
      navigate(builderPath(team.id, workflow.id));
    },
    onError: notifyError,
  });
  return (
    <TeamSection
      id="team-workflows"
      title="Workflows"
      actions={
        <Button
          size="sm"
          variant="outline"
          disabled={first === undefined || workflows === undefined || create.isPending}
          title={
            first === undefined ? "Add a member first: a workflow's steps need one." : undefined
          }
          onClick={() => first && create.mutate(first.dot_id)}
        >
          <Plus aria-hidden /> New workflow
        </Button>
      }
    >
      {workflows?.length === 0 && (
        <p className="text-muted-foreground">
          No workflows yet. A workflow says which member does what, in which order, and where
          you check the work before it ships.
        </p>
      )}
      {(workflows ?? []).length > 0 && (
        <ul className="flex flex-col gap-2">
          {workflows!.map((workflow) => (
            <WorkflowRow
              key={workflow.id}
              workflow={workflow}
              team={team}
              onRun={() => setRunning(workflow)}
            />
          ))}
        </ul>
      )}
      {running && (
        <RunNowDialog
          workflow={running}
          projects={team.projects}
          onClose={() => setRunning(null)}
        />
      )}
    </TeamSection>
  );
}

/// Why "Run now" is off for `workflow` of `team`, or undefined when it may start.
function cannotRun(workflow: Workflow, team: Team): string | undefined {
  if (workflow.problem !== null) return `It cannot start: ${workflow.problem}`;
  if (!workflow.enabled) return "Turn it on first.";
  if (team.projects.length === 0) return "Add a project to the team first.";
  return undefined;
}

function WorkflowRow(props: { workflow: Workflow; team: Team; onRun: () => void }) {
  const { workflow, team } = props;
  const qc = useQueryClient();
  const name = workflow.name;
  const toggle = useMutation({
    mutationFn: (enabled: boolean) =>
      commands.workflowUpdate(workflow.id, {
        name: workflow.name,
        definition: workflow.definition,
        enabled,
      }),
    onSuccess: () => {
      invalidate(qc, keys.workflows(team.id));
      invalidate(qc, keys.workflow(workflow.id));
    },
    onError: notifyError,
  });
  const blocked = cannotRun(workflow, team);
  const last = workflow.last_run;
  return (
    <ItemRow
      label={name}
      actions={
        <>
          <label className="flex items-center gap-2 text-xs">
            <Switch
              aria-label={`${name} is on`}
              checked={workflow.enabled}
              disabled={toggle.isPending}
              onCheckedChange={(on) => toggle.mutate(on)}
            />
            On
          </label>
          <Button
            size="sm"
            variant="outline"
            aria-label={`Run ${name} now`}
            disabled={blocked !== undefined}
            title={blocked}
            onClick={props.onRun}
          >
            Run now
          </Button>
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Link to={builderPath(team.id, workflow.id)} className="font-medium hover:underline">
          {name}
        </Link>
        <span className="text-xs text-muted-foreground">
          {TRIGGER_LABEL[workflow.definition.trigger.kind]}
        </span>
        {workflow.runs_in_flight > 0 && (
          <span className="text-xs text-muted-foreground">
            {plural(workflow.runs_in_flight, "run")} in flight
          </span>
        )}
      </div>
      {workflow.problem !== null && (
        <span className="text-xs text-destructive">{workflow.problem}</span>
      )}
      <span className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {last === null ? (
          "Not run yet"
        ) : (
          <>
            Last run
            <WorkflowRunBadge status={last.status} />
            {last.test && <TestBadge />}
            <Link
              to={`/workflow-runs/${last.id}`}
              aria-label={`The last run of ${name}`}
              className="hover:underline"
            >
              {relativeTime(last.started_at)}
            </Link>
          </>
        )}
      </span>
    </ItemRow>
  );
}
