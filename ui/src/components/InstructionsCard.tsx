import { useMutation, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { InstructionProposal } from "@/api/types";
import { Button } from "@/components/ui/button";
import { lineDiff, type DiffLine } from "@/lib/line-diff";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

const MARK: Record<DiffLine["kind"], string> = { same: "  ", added: "+ ", removed: "- " };
const LINE: Record<DiffLine["kind"], string> = {
  same: "",
  added: "bg-success/10 text-foreground",
  removed: "bg-destructive/10 text-foreground",
};

/// Instructions a dot proposed for itself (conversational chat spec section 4): why, its
/// instructions before and after line by line, and Approve or Reject. Nothing changes until the
/// person approves; the dot reads the answer as its chat's next message.
export function InstructionsCard(props: {
  proposal: InstructionProposal;
  dotName: string;
  /// The dot still works on the proposal's topic. The core refuses a decision until the topic's
  /// runs have ended, so Approve and Reject wait too.
  busy?: boolean;
}) {
  const { proposal, dotName, busy = false } = props;
  const qc = useQueryClient();
  const decide = useMutation({
    mutationFn: (approve: boolean) => commands.instructionProposalDecide(proposal.id, approve),
    onSuccess: (_answer, approve) => {
      invalidate(qc, keys.instructionProposals(proposal.dot_id));
      invalidate(qc, keys.inbox);
      // The dot's instructions changed, and no runtime event announces a dot change.
      if (approve) invalidateDots(qc);
    },
    onError: notifyError,
  });
  // One decision per card: after a success the card stays until its list has reloaded.
  const decided = decide.isPending || decide.isSuccess;
  const lines = lineDiff(proposal.base, proposal.instructions);
  return (
    <article
      aria-label={`New instructions for ${dotName}`}
      className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
    >
      <header className="flex flex-col gap-0.5">
        <span className="font-semibold text-needs">{dotName} wants to change its instructions</span>
        <span className="text-sm">{proposal.reason}</span>
      </header>
      <pre
        aria-label="Changes to the instructions"
        className="max-h-72 overflow-auto rounded-md bg-background/60 py-1 text-xs whitespace-pre-wrap"
      >
        {lines.map((line, i) => (
          <div key={i} className={`px-2 ${LINE[line.kind]}`} data-kind={line.kind}>
            {MARK[line.kind]}
            {line.text === "" ? " " : line.text}
          </div>
        ))}
      </pre>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          disabled={decided || busy}
          onClick={onSingleClick(() => decide.mutate(true))}
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={decided || busy}
          onClick={onSingleClick(() => decide.mutate(false))}
        >
          Reject
        </Button>
        {/* A disabled button gets no hover, so the reason is a line, not a tooltip. */}
        {busy && (
          <span className="text-xs text-muted-foreground">
            {dotName} is still working on this topic
          </span>
        )}
      </div>
    </article>
  );
}
