import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { toast } from "sonner";
import { Link, useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Team, Workflow } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { ItemRow, plural } from "@/screens/library/parts";

import { TRIGGER_LABEL, uniqueName } from "./format";
import { NewWorkflowDialog } from "./NewWorkflowDialog";
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
  const [creating, setCreating] = useState(false);
  // A workflow from a file someone handed over: it comes in turned off, for the builder.
  const bringIn = useMutation({
    mutationFn: async () => {
      const picked = await open({
        title: "Choose a workflow's file",
        filters: [{ name: "Sage workflow", extensions: ["json"] }],
      });
      return typeof picked === "string" ? commands.workflowImport(team.id, picked) : null;
    },
    onSuccess: (workflow) => {
      if (!workflow) return;
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
        <>
          <Button
            size="sm"
            variant="ghost"
            disabled={workflows === undefined || bringIn.isPending}
            title="Bring in a workflow from a file someone gave you"
            onClick={() => bringIn.mutate()}
          >
            Import
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={first === undefined || workflows === undefined}
            title={
              first === undefined ? "Add a member first: a workflow's steps need one." : undefined
            }
            onClick={() => setCreating(true)}
          >
            <Plus aria-hidden /> New workflow
          </Button>
        </>
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
              names={workflows!.map((w) => w.name)}
              onRun={() => setRunning(workflow)}
            />
          ))}
        </ul>
      )}
      {creating && (
        <NewWorkflowDialog
          team={team}
          workflows={workflows ?? []}
          onClose={() => setCreating(false)}
        />
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
function cannotRun(workflow: Workflow): string | undefined {
  if (workflow.problem !== null) return `It cannot start: ${workflow.problem}`;
  if (!workflow.enabled) return "Turn it on first.";
  return undefined;
}

function WorkflowRow(props: {
  workflow: Workflow;
  team: Team;
  /// The names of the team's workflows: a copy takes one that is free.
  names: string[];
  onRun: () => void;
}) {
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
  const blocked = cannotRun(workflow);
  const last = workflow.last_run;
  // A copy to change without touching this one: turned off, with a name of its own.
  const copy = useMutation({
    mutationFn: () =>
      commands.workflowCreateDraft(team.id, {
        name: uniqueName(`${workflow.name} copy`, props.names),
        definition: workflow.definition,
        enabled: false,
      }),
    onSuccess: () => invalidate(qc, keys.workflows(team.id)),
    onError: notifyError,
  });
  // A file to hand to someone else: its members go by name, since ids mean nothing elsewhere.
  const sendOut = useMutation({
    mutationFn: async () => {
      const picked = await save({
        title: `Save ${name} as a file`,
        // A name may hold what a file's name may not.
        defaultPath: `${name.replace(/[^A-Za-z0-9 _.-]+/g, "-")}.sage-workflow.json`,
        filters: [{ name: "Sage workflow", extensions: ["json"] }],
      });
      if (typeof picked !== "string") return null;
      await commands.workflowExport(workflow.id, picked);
      return picked;
    },
    onSuccess: (path) => {
      if (path) toast.success(`Saved ${name} to ${path}`);
    },
    onError: notifyError,
  });
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
            variant="ghost"
            aria-label={`Make a copy of ${name}`}
            disabled={copy.isPending}
            onClick={() => copy.mutate()}
          >
            Copy
          </Button>
          <Button
            size="sm"
            variant="ghost"
            aria-label={`Save ${name} as a file`}
            disabled={sendOut.isPending}
            onClick={() => sendOut.mutate()}
          >
            Export
          </Button>
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
