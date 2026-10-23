import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { Approval } from "@/api/types";
import { BrowserApprovalCard } from "@/components/BrowserApprovalCard";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { parkedNote } from "@/lib/approvals";
import { browserAsk } from "@/lib/browser";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { callLabel, toolPreview } from "@/lib/transcript";

type Decision = { approved: boolean; allowRule: string | null };

/// A call that waits for the user. A call of the built-in browser that Dots asked about has a
/// card of its own (`BrowserApprovalCard`); every other call has the tool card.
export function ApprovalCard(props: { approval: Approval; dotName?: string }) {
  const ask = browserAsk(props.approval);
  return ask ? (
    <BrowserApprovalCard approval={props.approval} ask={ask} dotName={props.dotName} />
  ) : (
    <ToolApprovalCard approval={props.approval} dotName={props.dotName} />
  );
}

/// A tool call that waits for the user: what the dot wants to do, and Approve or Deny with an
/// optional note the dot gets to read. Where the core offers rules for the call, an approval
/// can also save one of them to the dot's policy, so the same call is not asked again.
function ToolApprovalCard({ approval, dotName }: { approval: Approval; dotName?: string }) {
  const qc = useQueryClient();
  const [note, setNote] = useState("");
  const [choosing, setChoosing] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);
  // The core derives the rules from this call and accepts no other, so the card shows only
  // what it is given and never builds a pattern itself.
  const rules = useQuery({
    queryKey: keys.approvalRules(approval.id),
    queryFn: () => commands.approvalRuleCandidates(approval.id),
  });
  const offered = rules.data ?? [];
  // The narrowest rule comes first and is the one chosen until the user picks another.
  const chosen = picked ?? offered[0]?.pattern ?? null;
  const decide = useMutation({
    mutationFn: ({ approved, allowRule }: Decision) =>
      commands.decideApproval(approval.id, approved, note.trim() || null, allowRule),
    onSuccess: (_answer, { allowRule }) => {
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(approval.run_id));
      // A saved rule changed the dot's policy, and no runtime event announces a dot change.
      if (allowRule !== null) invalidateDots(qc);
    },
    onError: notifyError,
  });
  // One decision per card: after a success the card stays until its list has reloaded, and its
  // buttons stay off for that time.
  const decided = decide.isPending || decide.isSuccess;
  const noteId = `approval-note-${approval.id}`;
  const rulesId = `approval-rules-${approval.id}`;
  const tool = callLabel(approval.tool, approval.input);
  return (
    <article
      aria-label={`Approval for ${tool}`}
      className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
    >
      <header className="flex items-baseline gap-2">
        <span className="font-semibold text-needs">{tool}</span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs">
          {toolPreview(approval.input)}
        </span>
      </header>
      <pre className="max-h-60 overflow-auto rounded-md bg-background/60 p-2 text-xs whitespace-pre-wrap">
        {JSON.stringify(approval.input, null, 2)}
      </pre>
      {approval.parked && <p className="text-xs text-muted-foreground">{parkedNote(approval)}</p>}
      <div className="flex flex-wrap items-center gap-2">
        <label htmlFor={noteId} className="sr-only">
          Note for the dot
        </label>
        <Input
          id={noteId}
          className="min-w-40 flex-1"
          placeholder="Note for the dot (optional)"
          value={note}
          onChange={(e) => setNote(e.target.value)}
        />
        <Button
          size="sm"
          disabled={decided}
          onClick={onSingleClick(() => decide.mutate({ approved: true, allowRule: null }))}
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={decided}
          onClick={onSingleClick(() => decide.mutate({ approved: false, allowRule: null }))}
        >
          Deny
        </Button>
        {offered.length > 0 && (
          <Button
            size="sm"
            variant="outline"
            aria-expanded={choosing}
            aria-controls={rulesId}
            disabled={decided}
            onClick={() => setChoosing((open) => !open)}
          >
            Approve and always allow
          </Button>
        )}
      </div>
      {rules.isError && (
        <p className="text-xs text-muted-foreground">
          The rules that could be saved with this approval could not be loaded. You can still
          approve or deny this call.
        </p>
      )}
      {choosing && chosen !== null && (
        <fieldset id={rulesId} className="flex flex-col gap-2 rounded-md border p-2">
          <legend className="px-1 text-xs font-medium">
            Always allow for {dotName ?? "this dot"}
          </legend>
          {offered.map((rule) => (
            <label key={rule.pattern} className="flex items-baseline gap-2 text-xs">
              <input
                type="radio"
                name={rulesId}
                value={rule.pattern}
                checked={chosen === rule.pattern}
                onChange={() => setPicked(rule.pattern)}
              />
              <code className="font-mono">{rule.pattern}</code>
              <span className="text-muted-foreground">{rule.label}</span>
            </label>
          ))}
          <p className="text-xs text-muted-foreground">A deny rule still wins over this rule.</p>
          <div className="flex gap-2">
            <Button
              size="sm"
              disabled={decided}
              onClick={onSingleClick(() => decide.mutate({ approved: true, allowRule: chosen }))}
            >
              Approve and save rule
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setChoosing(false)}>
              Cancel
            </Button>
          </div>
        </fieldset>
      )}
    </article>
  );
}
