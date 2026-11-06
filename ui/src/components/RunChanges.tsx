import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Redo2, Undo2 } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { FileAction, PlannedFile, Run, RunChanges, UndoOutcome } from "@/api/types";
import { DiffView } from "@/components/DiffView";
import { ErrorNote } from "@/components/ErrorNote";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { isActive } from "@/lib/home-topics";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// Whether a run worked in a folder of the person's (not a worktree of its own): only such a
/// run has a checkpoint (undo spec section 2).
export function isFolderRun(run: Run): boolean {
  return run.branch === null && run.workspace_path !== null;
}

function files(n: number): string {
  return n === 1 ? "1 file" : `${n} files`;
}

/// A folder run's changes, read once the run ended. `null` data: the run has no checkpoint.
export function useRunChanges(run: Run) {
  return useQuery({
    queryKey: [...keys.changes(run.id), run.status],
    queryFn: () => commands.runChanges(run.id),
    enabled: isFolderRun(run) && !isActive(run),
  });
}

const ACTION_TEXT: Record<FileAction, string> = {
  ready: "Will be put back",
  conflict: "Changed since",
  done: "Already back",
  cannot: "Can't be put back",
};

const ACTION_VARIANT: Record<FileAction, "secondary" | "destructive" | "outline"> = {
  ready: "secondary",
  conflict: "destructive",
  done: "outline",
  cannot: "outline",
};

function PlannedRow({ file }: { file: PlannedFile }) {
  return (
    <li className="flex flex-col gap-0.5 py-1">
      <div className="flex items-center gap-2">
        <Badge variant={ACTION_VARIANT[file.action]}>{ACTION_TEXT[file.action]}</Badge>
        <span className="text-xs text-muted-foreground">{file.status}</span>
        <span className="font-mono text-xs break-all">{file.path}</span>
      </div>
      {file.reason && <p className="text-xs text-muted-foreground">{file.reason}</p>}
    </li>
  );
}

function outcomeText(outcome: UndoOutcome): string {
  const verb = outcome.redo ? "Redid" : "Undid";
  const left =
    outcome.skipped.length > 0 ? ` Left ${files(outcome.skipped.length)} as they are.` : "";
  return `${verb} the run's changes: put back ${files(outcome.restored.length)}.${left}`;
}

/// "Undo these changes" (or "Redo" once undone): the run's files with what would happen to each
/// now, a warning when later runs changed the same files, and a box to overwrite files that
/// changed after the run. Nothing is written before the person confirms.
export function UndoDialog(props: {
  runId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const qc = useQueryClient();
  const [overwrite, setOverwrite] = useState(false);
  const preview = useQuery({
    queryKey: keys.changesPreview(props.runId),
    queryFn: () => commands.runChangesPreview(props.runId),
    enabled: props.open,
    // What is on disk changes outside Dots: read it again each time the dialog opens.
    staleTime: 0,
    gcTime: 0,
  });
  const apply = useMutation({
    mutationFn: (redo: boolean) =>
      redo ? commands.redoRun(props.runId, overwrite) : commands.undoRun(props.runId, overwrite),
    onSuccess: (outcome) => {
      toast.success(outcomeText(outcome));
      props.onOpenChange(false);
      setOverwrite(false);
    },
    onSettled: () => {
      invalidate(qc, keys.changes(props.runId));
      invalidate(qc, keys.events(props.runId));
    },
  });
  const data = preview.data;
  const redo = data?.redo ?? false;
  const conflicts = data?.files.filter((f) => f.action === "conflict") ?? [];
  const ready = data?.files.filter((f) => f.action === "ready") ?? [];
  const toWrite = ready.length + (overwrite ? conflicts.length : 0);
  const title = redo ? "Redo these changes?" : "Undo these changes?";
  return (
    <Dialog
      open={props.open}
      onOpenChange={(open) => {
        if (!open) setOverwrite(false);
        props.onOpenChange(open);
      }}
    >
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {redo
              ? "Dots puts these files back as the run left them."
              : "Dots puts these files back as they were before the run: files it made are deleted, files it deleted come back."}{" "}
            You can {redo ? "undo" : "redo"} this afterwards.
          </DialogDescription>
        </DialogHeader>
        {preview.isPending && props.open && (
          <p className="text-sm text-muted-foreground">Looking at the files…</p>
        )}
        {preview.isError && <ErrorNote error={preview.error} />}
        {data && (
          <div className="flex flex-col gap-3">
            {data.later_runs.length > 0 && (
              <div
                role="alert"
                className="rounded-md border border-needs-border bg-needs-surface p-2 text-sm"
              >
                Later runs in this folder changed some of the same files:{" "}
                {data.later_runs.map((id, i) => (
                  <span key={id}>
                    {i > 0 && ", "}
                    <Link to={`/runs/${id}`} className="underline">
                      this run
                    </Link>
                  </span>
                ))}
                . Putting these files back also takes back what they did to them.
              </div>
            )}
            <ul aria-label="Files" className="divide-y">
              {data.files.map((file) => (
                <PlannedRow key={file.path} file={file} />
              ))}
            </ul>
            {conflicts.length > 0 && (
              <label className="flex items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="mt-1"
                  checked={overwrite}
                  onChange={(e) => setOverwrite(e.target.checked)}
                />
                <span>
                  Also overwrite the {files(conflicts.length)} changed after the run. Without
                  this, Dots leaves {conflicts.length === 1 ? "it" : "them"} as{" "}
                  {conflicts.length === 1 ? "it is" : "they are"}.
                </span>
              </label>
            )}
          </div>
        )}
        {apply.isError && (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(apply.error)}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => props.onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            disabled={!data || toWrite === 0 || apply.isPending}
            onClick={onSingleClick(() => apply.mutate(redo))}
          >
            {redo ? "Redo" : "Undo"} {files(toWrite)}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The button that opens `UndoDialog`, labelled for where the run's changes stand.
export function UndoButton(props: { runId: string; changes: RunChanges; size?: "xs" | "sm" }) {
  const [open, setOpen] = useState(false);
  const undone = props.changes.undone_at !== null;
  if (!props.changes.files.some((f) => f.undoable)) return null;
  return (
    <>
      <Button size={props.size ?? "xs"} variant="outline" onClick={() => setOpen(true)}>
        {undone ? <Redo2 aria-hidden /> : <Undo2 aria-hidden />}
        {undone ? "Redo these changes" : "Undo these changes"}
      </Button>
      <UndoDialog runId={props.runId} open={open} onOpenChange={setOpen} />
    </>
  );
}

/// Under a run's turn in the conversation: how many files it changed, and Undo (or Redo). Nothing
/// for a run that changed no file, has no checkpoint, or is still working.
export function ChangesLine({ run }: { run: Run }) {
  const changes = useRunChanges(run);
  const data = changes.data;
  if (!data || data.state !== "recorded" || data.files.length === 0) return null;
  const undone = data.undone_at !== null;
  return (
    <div role="group" aria-label="Changed files" className="flex flex-wrap items-center gap-2">
      <Link to={`/runs/${run.id}`} className="text-xs text-muted-foreground hover:underline">
        Changed {files(data.files.length)}
        {undone && " · undone"}
      </Link>
      <UndoButton runId={run.id} changes={data} />
    </div>
  );
}

/// The Changes of one folder run on its page: each changed file with its diff, why a file
/// can't be put back, and Undo (or Redo).
export function RunChangesPanel({ run }: { run: Run }) {
  const changes = useRunChanges(run);
  const recorded = changes.data?.state === "recorded";
  const preview = useQuery({
    queryKey: keys.changesPreview(run.id),
    queryFn: () => commands.runChangesPreview(run.id),
    enabled: recorded && (changes.data?.files.length ?? 0) > 0,
  });
  if (isActive(run)) {
    return <p className="text-muted-foreground">Dots lists the run's changes once it ends.</p>;
  }
  if (changes.isError) return <ErrorNote error={changes.error} />;
  const data = changes.data;
  if (data === undefined) return null;
  if (data === null) {
    return (
      <p className="text-muted-foreground">
        Dots kept no record of this run's changes: it ran before Dots kept them.
      </p>
    );
  }
  if (data.state !== "recorded") {
    return (
      <p className="text-muted-foreground">{data.note ?? "This run's changes were not recorded."}</p>
    );
  }
  const cannot = data.files.filter((f) => !f.undoable);
  return (
    <section aria-label={`Changes of run ${run.id}`} className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm">
          {data.files.length === 0 ? "No files changed." : `Changed ${files(data.files.length)}`}
          {data.undone_at !== null && " · undone"}
        </span>
        <UndoButton runId={run.id} changes={data} size="sm" />
      </div>
      {data.note && <p className="text-sm text-muted-foreground">{data.note}</p>}
      {data.git && data.files.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Files your repository ignores are not tracked.
        </p>
      )}
      {cannot.length > 0 && (
        <ul aria-label="Can't be undone" className="text-sm">
          {cannot.map((f) => (
            <li key={f.path}>
              <span className="font-mono text-xs">{f.path}</span>
              {f.reason && <span className="text-muted-foreground"> — {f.reason}</span>}
            </li>
          ))}
        </ul>
      )}
      {preview.isError && <ErrorNote error={preview.error} />}
      {preview.data && <DiffView diff={preview.data.diff} />}
    </section>
  );
}
