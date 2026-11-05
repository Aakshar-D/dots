import {
  hashKey,
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { Step, Team, Workflow, WorkflowSpec } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { useDebounced } from "@/lib/use-debounced";
import { plural } from "@/screens/library/parts";

import { RunNowDialog } from "../RunNowDialog";
import { BudgetCard, TriggerCard } from "./cards";
import {
  deleteStep,
  findStep,
  insertStep,
  type ListRef,
  moveStep,
  newStep,
  type StepKindName,
  TOP,
  updateStep,
} from "./draft";
import type { NameOf } from "./labels";
import { type Builder, BuilderProvider, StepList } from "./StepList";
import { StepSheet } from "./StepSheet";
import { TriggerSheet } from "./TriggerSheet";

/// How long the draft must stay unchanged before the live check asks the core about it.
export const CHECK_DELAY_MS = 300;

/// The builder (teams spec section 10.2, layout B) at `/teams/:teamId/workflows/:workflowId`.
export function WorkflowBuilderScreen() {
  const { teamId = "", workflowId = "" } = useParams();
  const team = useQuery({ queryKey: keys.team(teamId), queryFn: () => commands.teamGet(teamId) });
  const workflow = useQuery({
    queryKey: keys.workflow(workflowId),
    queryFn: () => commands.workflowGet(workflowId),
  });
  const failed = team.isError ? team.error : workflow.isError ? workflow.error : undefined;
  if (failed !== undefined) {
    return (
      <Page title="Workflow">
        <ErrorNote error={failed} />
      </Page>
    );
  }
  if (!team.data || !workflow.data) return <Page title="Workflow" />;
  return <WorkflowBuilder key={workflowId} team={team.data} workflow={workflow.data} />;
}

/// What a workflow saves: the builder's draft starts from it.
const specOf = (w: Workflow | WorkflowSpec): WorkflowSpec => ({
  name: w.name,
  definition: w.definition,
  enabled: w.enabled,
});

/// Whether two specs say the same, whatever the order of their fields.
const same = (a: WorkflowSpec, b: WorkflowSpec) => hashKey([a]) === hashKey([b]);

/// The draft as Save and the live check send it: its name trimmed.
const sendable = (draft: WorkflowSpec): WorkflowSpec => ({ ...draft, name: draft.name.trim() });

/// The builder edits a draft of the definition in React state. Save sends it with
/// `workflow_update`; the live check sends it (debounced) to `workflow_check_spec` and puts the
/// problem on the card whose step it names, else on the header. Save is off while the draft is
/// unchanged, has a problem, is being checked, or is being saved.
function WorkflowBuilder({ team, workflow }: { team: Team; workflow: Workflow }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const stored = specOf(workflow);
  // `base` is the saved workflow the draft was made from or has caught up with.
  const [base, setBase] = useState(stored);
  const [draft, setDraft] = useState(stored);
  const [refusal, setRefusal] = useState<string>();
  const [openId, setOpenId] = useState<string | null>(null);
  const [triggerOpen, setTriggerOpen] = useState(false);
  const [testing, setTesting] = useState<Workflow | null>(null);
  const cards = useRef(new Map<string, HTMLElement>());
  const [focusId, setFocusId] = useState<string | null>(null);

  // The workflow saved elsewhere (or answered by our own save): an untouched draft follows it,
  // an edited one is kept and is now compared with it.
  const storedKey = hashKey([stored]);
  useEffect(() => {
    setDraft((d) => (same(d, base) ? stored : d));
    setBase(stored);
    // Keyed by what the saved workflow says: `stored` and `base` are read as they are then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [storedKey]);

  // Focus follows a card that moved or was added.
  useEffect(() => {
    if (focusId === null) return;
    cards.current.get(focusId)?.focus();
    setFocusId(null);
  }, [focusId]);

  const sent = useMemo(() => sendable(draft), [draft]);
  const settled = useDebounced(sent, CHECK_DELAY_MS);
  const check = useQuery({
    queryKey: keys.workflowProblem(team.id, settled),
    queryFn: () => commands.workflowCheckSpec(team.id, settled),
    // The last problem stays on its card while the next draft is checked.
    placeholderData: keepPreviousData,
  });
  const problem = check.data ?? null;
  const checking = !same(settled, sent) || check.isPending || check.isPlaceholderData;
  const onStep = problem?.step_id ? findStep(draft.definition.steps, problem.step_id) : undefined;
  const changed = !same(sent, base);
  // A check that could not be made does not hold Save back: the save checks again.
  const blocked = checking
    ? "The draft is being checked."
    : problem
      ? `Fix this first: ${problem.message}`
      : undefined;

  const save = useMutation({
    mutationFn: (spec: WorkflowSpec) => commands.workflowUpdate(workflow.id, spec),
    onSuccess: (saved, spec) => {
      qc.setQueryData(keys.workflow(saved.id), saved);
      // Edits made while it was saving stay in the draft.
      setDraft((d) => (same(sendable(d), spec) ? specOf(saved) : d));
      setRefusal(undefined);
      invalidate(qc, keys.workflows(team.id));
    },
    onError: (e) => setRefusal(errorMessage(e)),
  });

  const remove = useMutation({
    mutationFn: () => commands.workflowDelete(workflow.id),
    onSuccess: () => {
      navigate(`/teams/${team.id}`);
      invalidate(qc, keys.workflows(team.id));
      invalidate(qc, ["workflow-runs"]);
    },
    onError: notifyError,
  });

  /// Every edit goes through here: a refusal of the last save is about a draft that is gone.
  function edit(change: (d: WorkflowSpec) => WorkflowSpec) {
    setDraft(change);
    setRefusal(undefined);
  }
  const editSteps = (change: (steps: Step[]) => Step[]) =>
    edit((d) => ({ ...d, definition: { ...d.definition, steps: change(d.definition.steps) } }));

  const names = new Map(team.members.map((m) => [m.dot_id, m.dot_name]));
  const nameOf: NameOf = (id) => names.get(id) ?? id;

  const builder: Builder = {
    steps: draft.definition.steps,
    nameOf,
    problem: onStep ? problem : null,
    add: (list: ListRef, index: number, kind: StepKindName) => {
      const step = newStep(kind, draft.definition.steps, team);
      editSteps((steps) => insertStep(steps, list, index, step));
      setFocusId(step.id);
    },
    move: (id, by) => {
      editSteps((steps) => moveStep(steps, id, by));
      setFocusId(id);
    },
    remove: (id) => editSteps((steps) => deleteStep(steps, id)),
    open: setOpenId,
    card: (id, el) => {
      if (el) cards.current.set(id, el);
      else cards.current.delete(id);
    },
  };

  // A Test run starts while the workflow is off (Task 10's call), so only a problem, a check
  // under way or a team with no project holds it back.
  const testBlocked =
    blocked ?? (team.projects.length === 0 ? "Add a project to the team first." : undefined);
  /// Test run saves the draft first (section 5.1: it runs the saved definition), then asks for
  /// the task and the project.
  async function testRun() {
    const saved = changed ? await save.mutateAsync(sent).catch(() => null) : workflow;
    if (saved) setTesting(saved);
  }

  const inFlight = workflow.runs_in_flight;
  const open = openId ? findStep(draft.definition.steps, openId) : undefined;
  return (
    <div className="flex flex-col gap-4 p-6">
      <header className="flex flex-col gap-2">
        <Link
          to={`/teams/${team.id}`}
          className="w-fit text-xs text-muted-foreground hover:underline"
        >
          {team.name}
        </Link>
        <h1 className="sr-only">{workflow.name}</h1>
        <div className="flex flex-wrap items-center gap-3">
          <Input
            aria-label="Workflow name"
            className="h-9 max-w-sm flex-1 text-lg font-semibold"
            value={draft.name}
            onChange={(e) => {
              const name = e.target.value;
              edit((d) => ({ ...d, name }));
            }}
          />
          <span className="flex-1" />
          <label className="flex items-center gap-2 text-xs">
            <Switch
              aria-label="On"
              checked={draft.enabled}
              onCheckedChange={(enabled) => edit((d) => ({ ...d, enabled }))}
            />
            On
          </label>
          <Button
            size="sm"
            variant="outline"
            disabled={testBlocked !== undefined || save.isPending}
            title={testBlocked}
            onClick={() => void testRun()}
          >
            Test run
          </Button>
          <Button
            size="sm"
            disabled={!changed || blocked !== undefined || save.isPending}
            title={changed ? blocked : "Nothing has changed."}
            onClick={() => save.mutate(sent)}
          >
            Save
          </Button>
          <ConfirmButton
            title={`Delete ${workflow.name}?`}
            description={
              `Its runs stay on the team page, as runs of a deleted workflow. Runs in flight go ` +
              `on with the version they started with` +
              `${inFlight > 0 ? `: ${plural(inFlight, "run")} now` : ""}.`
            }
            confirmLabel="Delete workflow"
            disabled={remove.isPending}
            onConfirm={() => remove.mutate()}
          >
            Delete workflow
          </ConfirmButton>
        </div>
        {refusal && (
          <p role="alert" className="text-sm text-destructive">
            {refusal}
          </p>
        )}
        {problem && !onStep && (
          <p role="alert" className="text-sm text-destructive">
            {problem.message}
          </p>
        )}
        {check.isError && (
          <p role="alert" className="text-sm text-destructive">
            The draft could not be checked: {errorMessage(check.error)}
          </p>
        )}
        {inFlight > 0 && (
          <p role="status" className="text-xs text-muted-foreground">
            {inFlight === 1
              ? "1 run in flight uses the previous version"
              : `${inFlight} runs in flight use the previous version`}
          </p>
        )}
      </header>
      <div className="mx-auto flex w-full max-w-2xl flex-col items-stretch">
        <TriggerCard
          trigger={draft.definition.trigger}
          nameOf={nameOf}
          onOpen={() => setTriggerOpen(true)}
        />
        <BuilderProvider value={builder}>
          <StepList steps={draft.definition.steps} list={TOP} prefix="" name="the workflow" top />
        </BuilderProvider>
        <div className="pt-4">
          <BudgetCard
            budget={draft.definition.budget}
            onChange={(budget) => edit((d) => ({ ...d, definition: { ...d.definition, budget } }))}
          />
        </div>
      </div>
      <StepSheet
        step={open}
        steps={draft.definition.steps}
        team={team}
        nameOf={nameOf}
        onChange={(step) => editSteps((steps) => updateStep(steps, step.id, () => step))}
        onClose={() => setOpenId(null)}
      />
      <TriggerSheet
        open={triggerOpen}
        trigger={draft.definition.trigger}
        enabled={draft.enabled}
        workflow={workflow}
        team={team}
        nameOf={nameOf}
        onChange={(trigger) => edit((d) => ({ ...d, definition: { ...d.definition, trigger } }))}
        onClose={() => setTriggerOpen(false)}
      />
      {testing && (
        <RunNowDialog
          workflow={testing}
          projects={team.projects}
          test
          onClose={() => setTesting(null)}
        />
      )}
    </div>
  );
}
