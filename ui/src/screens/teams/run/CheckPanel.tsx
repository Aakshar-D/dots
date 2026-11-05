import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { CommandError, commands } from "@/api/commands";
import type { CheckView, WorkflowRun } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Markdown } from "@/components/Markdown";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { settle, StopRun, ViewDiffButton } from "./actions";
import { sendBackTarget } from "./steps";

/// The check a workflow run waits on (section 5.4), with what the person does about it: its
/// hand-off summary, View diff for each branch it shows, a note, Send back (to the step it sends
/// back to; needs the note), Approve and, unless `stop` is false, Stop. Approve sends the
/// branches and commits it showed, and Approve and Send back the step run it showed: when git
/// moved since or the run waits on another check, the core refuses, the refusal is shown and the
/// check is read again. Both wait while the check is read, and once one answers the check it
/// showed is dropped, so a later check never shows this one's view. Shown only while the run
/// waits on a check.
export function CheckPanel({ run, stop = true }: { run: WorkflowRun; stop?: boolean }) {
  const qc = useQueryClient();
  const waiting = run.status === "waiting" && run.waiting_on === "check";
  const check = useQuery({
    queryKey: keys.workflowCheck(run.id),
    queryFn: () => commands.workflowCheck(run.id),
    enabled: waiting,
  });
  const [note, setNote] = useState("");
  const [refusal, setRefusal] = useState<string>();
  // The check this answer was about is gone or changed: the next wait reads its own.
  const forget = () => qc.removeQueries({ queryKey: keys.workflowCheck(run.id), exact: true });
  const onError = (e: unknown) => {
    setRefusal(errorMessage(e));
    forget();
    // The check moved under the page (a branch changed, the run went on): read the run again.
    if (e instanceof CommandError && e.kind === "conflict") {
      invalidate(qc, keys.workflowRun(run.id));
    }
  };
  const done = (next: WorkflowRun) => {
    setNote("");
    setRefusal(undefined);
    forget();
    settle(qc, next);
  };
  const approve = useMutation({
    mutationFn: (shown: CheckView) => {
      const tips = shown.branches.map(({ branch, commit }) => ({ branch, commit }));
      return commands.workflowApprove(run.id, note.trim() || null, shown.step_run_id, tips);
    },
    onSuccess: done,
    onError,
  });
  const sendBack = useMutation({
    mutationFn: (shown: CheckView) =>
      commands.workflowSendBack(run.id, note.trim(), shown.step_run_id),
    onSuccess: done,
    onError,
  });
  if (!waiting) return null;

  const stepId = check.data?.step_id ?? run.current_step ?? "";
  const target = sendBackTarget(run.definition.steps, stepId);
  // Approve and Send back name the check on screen: none while it is read (again).
  const shown = check.isFetching ? undefined : check.data;
  const busy = approve.isPending || sendBack.isPending;
  const noteId = `check-note-${run.id}`;
  return (
    <section
      aria-label="Your check"
      className="flex flex-col gap-3 rounded-lg border border-needs-border bg-needs-surface p-3"
    >
      {check.isError ? (
        <ErrorNote error={check.error} />
      ) : !check.data ? (
        <p className="text-muted-foreground">Reading the check…</p>
      ) : (
        <>
          {check.data.summary.trim() ? (
            <div className="text-sm">
              <Markdown text={check.data.summary} />
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">The step before it said nothing.</p>
          )}
          {check.data.branches.length === 0 ? (
            <p className="text-sm text-muted-foreground">It shows no branch: nothing changed.</p>
          ) : (
            <ul aria-label="Branches to check" className="flex flex-col gap-1">
              {check.data.branches.map((b) => (
                <li
                  key={b.branch}
                  aria-label={b.branch}
                  className="flex flex-wrap items-center gap-2 text-sm"
                >
                  <span className="min-w-0 flex-1 font-mono text-xs break-all">{b.branch}</span>
                  {!b.commit && <span className="text-xs text-muted-foreground">Gone</span>}
                  <ViewDiffButton runId={run.id} branch={b.branch} commit={b.commit} />
                </li>
              ))}
            </ul>
          )}
        </>
      )}
      <Field
        label="Note"
        htmlFor={noteId}
        hint="Send back needs one: it tells the step what to change. Approve keeps it with the approval."
      >
        <Textarea
          id={noteId}
          rows={3}
          value={note}
          placeholder="What should change?"
          onChange={(e) => setNote(e.target.value)}
        />
      </Field>
      {refusal && (
        <p role="alert" className="text-sm text-destructive">
          {refusal}
        </p>
      )}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={busy || !shown}
          onClick={() => shown && approve.mutate(shown)}
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={busy || !shown || note.trim() === ""}
          title={note.trim() === "" ? "Write a note first: it says what to change." : undefined}
          onClick={() => shown && sendBack.mutate(shown)}
        >
          {target ? `Send back to ${target.name.trim() || target.id}` : "Send back"}
        </Button>
        {stop && <StopRun run={run} />}
      </div>
    </section>
  );
}
