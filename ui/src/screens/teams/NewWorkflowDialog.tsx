import { useQueryClient } from "@tanstack/react-query";
import { useId, useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Step, Team, Workflow, WorkflowDraft, WorkflowSpec } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { KIND_LABEL } from "./builder/labels";
import { newWorkflowSpec, uniqueName } from "./format";
import { TEMPLATES, type WorkflowTemplate } from "./templates";

/// What the dialog is about to make: a workflow, and what the person still has to do with it.
type Made = { spec: WorkflowSpec; needs: string[] };

/// The steps of `steps` in run order, a branch's own steps after it: what a draft will do.
function flat(steps: Step[]): Step[] {
  return steps.flatMap((step) =>
    step.kind === "branch"
      ? [step, ...step.paths.flatMap((p) => flat(p.steps)), ...flat(step.otherwise)]
      : [step],
  );
}

/// "New workflow" (plain workflows spec sections 2 and 3): describe the job and Sage writes
/// the workflow, start from one that is ready, or start with an empty one. Whatever is chosen
/// is shown first, with what is left to do, and only then made, turned off, and opened in the
/// builder.
export function NewWorkflowDialog(props: {
  team: Team;
  workflows: Workflow[];
  onClose: () => void;
}) {
  const { team } = props;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const dialog = useId();
  const [description, setDescription] = useState("");
  const [made, setMade] = useState<Made>();
  const [writing, setWriting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const taken = props.workflows.map((w) => w.name);

  function show(spec: WorkflowSpec, needs: string[]) {
    setError(undefined);
    setMade({ spec: { ...spec, name: uniqueName(spec.name, taken), enabled: false }, needs });
  }

  async function write() {
    setWriting(true);
    setError(undefined);
    try {
      const draft: WorkflowDraft = await commands.workflowDraft(dialog, team.id, description);
      show({ name: draft.name, definition: draft.definition, enabled: false }, draft.notes);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWriting(false);
    }
  }

  async function make() {
    if (!made) return;
    setBusy(true);
    setError(undefined);
    try {
      const workflow = await commands.workflowCreateDraft(team.id, made.spec);
      invalidate(qc, keys.workflows(team.id));
      navigate(`/teams/${team.id}/workflows/${workflow.id}`);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  function close() {
    if (writing) void commands.workflowDraftCancel(dialog);
    props.onClose();
  }

  const pick = (template: WorkflowTemplate) => show(template.build(team), template.needs);
  return (
    <Dialog open onOpenChange={(next) => !next && close()}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>New workflow</DialogTitle>
          <DialogDescription>
            {made
              ? "This is what will be made. It starts turned off, and opens in the builder for you to finish."
              : "Say what you want done and Sage writes the workflow, or start from one that is ready."}
          </DialogDescription>
        </DialogHeader>
        {made ? (
          <section aria-label="The workflow to make" className="flex flex-col gap-3">
            <p className="font-semibold">{made.spec.name}</p>
            <ol aria-label="Its steps" className="flex flex-col gap-1 text-sm">
              {flat(made.spec.definition.steps).map((step, i) => (
                <li key={step.id} className="flex gap-2">
                  <span className="w-5 text-right text-muted-foreground">{i + 1}.</span>
                  <span className="min-w-0 flex-1">{step.name}</span>
                  <span className="text-xs text-muted-foreground">{KIND_LABEL[step.kind]}</span>
                </li>
              ))}
            </ol>
            {made.needs.length > 0 && (
              <div className="rounded-md border border-needs-border bg-needs-surface p-2 text-sm">
                <p className="font-medium text-needs">Before it can run</p>
                <ul aria-label="Before it can run" className="list-disc pl-5">
                  {made.needs.map((need, i) => (
                    <li key={i}>{need}</li>
                  ))}
                </ul>
              </div>
            )}
          </section>
        ) : (
          <>
            <Field
              label="Describe it"
              htmlFor="new-workflow-description"
              hint="In your own words: what starts it, what should be written or decided, what you want to approve, and where it should go."
            >
              <Textarea
                id="new-workflow-description"
                rows={4}
                placeholder="Every Monday, write a post about our week, let me approve it, and send it to our Telegram channel."
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </Field>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                disabled={writing || description.trim() === ""}
                onClick={() => void write()}
              >
                {writing ? "Writing…" : "Write it for me"}
              </Button>
              {writing && (
                <span role="status" className="text-xs text-muted-foreground">
                  This takes up to a minute.
                </span>
              )}
            </div>
            <section aria-labelledby="new-workflow-ready" className="flex flex-col gap-2">
              <h3 id="new-workflow-ready" className="text-sm font-medium">
                Or start from one that is ready
              </h3>
              <ul aria-label="Ready workflows" className="flex flex-col gap-2">
                {TEMPLATES.map((template) => (
                  <li key={template.id}>
                    <button
                      type="button"
                      disabled={writing}
                      onClick={() => pick(template)}
                      className="w-full rounded-md border bg-card p-2 text-left hover:bg-accent focus-visible:ring-2 focus-visible:outline-none disabled:opacity-50"
                    >
                      <span className="block text-sm font-medium">{template.name}</span>
                      <span className="block text-xs text-muted-foreground">{template.summary}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          {made ? (
            <>
              <Button type="button" variant="outline" disabled={busy} onClick={() => setMade(undefined)}>
                Back
              </Button>
              <Button type="button" disabled={busy} onClick={() => void make()}>
                Make it
              </Button>
            </>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="outline"
                disabled={writing}
                onClick={() =>
                  show(newWorkflowSpec("New workflow", team.members[0]?.dot_id ?? ""), [])
                }
              >
                Start with an empty one
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
