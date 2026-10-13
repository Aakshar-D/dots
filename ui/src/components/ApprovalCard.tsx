import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { Approval } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { notifyError } from "@/lib/notify";
import { keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { toolPreview } from "@/lib/transcript";

/// A tool call that waits for the user: what the dot wants to do, and Approve or Deny with an
/// optional note the dot gets to read.
export function ApprovalCard({ approval }: { approval: Approval }) {
  const qc = useQueryClient();
  const [note, setNote] = useState("");
  const decide = useMutation({
    mutationFn: (approved: boolean) =>
      commands.decideApproval(approval.id, approved, note.trim() || null),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: keys.inbox });
      void qc.invalidateQueries({ queryKey: keys.approvals(approval.run_id) });
    },
    onError: notifyError,
  });
  // One decision per card: after a success the card stays until its list has reloaded, and its
  // buttons stay off for that time.
  const decided = decide.isPending || decide.isSuccess;
  const noteId = `approval-note-${approval.id}`;
  return (
    <article
      aria-label={`Approval for ${approval.tool}`}
      className="flex flex-col gap-2 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3"
    >
      <header className="flex items-baseline gap-2">
        <span className="font-semibold">{approval.tool}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs">
          {toolPreview(approval.input)}
        </span>
      </header>
      <pre className="max-h-60 overflow-auto rounded-md bg-background/60 p-2 text-xs whitespace-pre-wrap">
        {JSON.stringify(approval.input, null, 2)}
      </pre>
      {approval.parked && (
        <p className="text-xs text-muted-foreground">
          The run stopped waiting and is parked. Approving continues it in a new run.
        </p>
      )}
      <div className="flex items-center gap-2">
        <label htmlFor={noteId} className="sr-only">
          Note for the dot
        </label>
        <Input
          id={noteId}
          placeholder="Note for the dot (optional)"
          value={note}
          onChange={(e) => setNote(e.target.value)}
        />
        <Button size="sm" disabled={decided} onClick={onSingleClick(() => decide.mutate(true))}>
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={decided}
          onClick={onSingleClick(() => decide.mutate(false))}
        >
          Deny
        </Button>
      </div>
    </article>
  );
}
