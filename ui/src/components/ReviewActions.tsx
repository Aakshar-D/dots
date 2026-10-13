import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// The two ways a finished run stops waiting for the user: "Mark reviewed", and "Discard
/// worktree", which asks first. The Inbox and the run view show the same two; each says which
/// of them its run can take.
export function ReviewActions(props: {
  runId: string;
  /// Named in the question a discard asks.
  branch: string | null;
  canReview: boolean;
  /// Only when the core says that the worktree exists.
  canDiscard: boolean;
}) {
  const qc = useQueryClient();
  // What a review changes. Also after a refusal: a run that is gone, or was reviewed or
  // discarded elsewhere, has to show as it is now and leave the Inbox.
  const reload = () => {
    for (const queryKey of [["thread"], ["diff"], keys.inbox]) invalidate(qc, queryKey);
  };
  const markReviewed = useMutation({
    mutationFn: () => commands.markReviewed(props.runId),
    onSuccess: () => void toast.success("Marked as reviewed"),
    onError: notifyError,
    onSettled: reload,
  });
  const discard = useMutation({
    mutationFn: () => commands.discardWorktree(props.runId),
    onSuccess: () => void toast.success("Worktree discarded"),
    onError: notifyError,
    onSettled: reload,
  });
  const busy = markReviewed.isPending || discard.isPending;
  return (
    <>
      {props.canReview && (
        <Button
          size="sm"
          variant="outline"
          disabled={busy}
          onClick={onSingleClick(() => markReviewed.mutate())}
        >
          Mark reviewed
        </Button>
      )}
      {props.canDiscard && (
        <ConfirmButton
          title="Discard this worktree?"
          description={
            <>
              The worktree folder and the branch <code>{props.branch}</code> are deleted, with
              every change the dot made there. This cannot be undone.
            </>
          }
          confirmLabel="Discard the worktree"
          onConfirm={() => discard.mutate()}
          disabled={busy}
        >
          Discard worktree
        </ConfirmButton>
      )}
    </>
  );
}
