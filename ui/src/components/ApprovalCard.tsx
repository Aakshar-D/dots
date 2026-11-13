import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleHelp, Send, ShieldAlert, Trash2 } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { Approval, ApprovalGuard } from "@/api/types";
import { BrowserApprovalCard } from "@/components/BrowserApprovalCard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useRunRefusal } from "@/lib/access";
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
  // A call that touches a private folder always asks: the core offers no rule for it, and the
  // card says why.
  const privateCall = useQuery({
    queryKey: keys.approvalPrivate(approval.id),
    queryFn: () => commands.approvalTouchesPrivate(approval.id),
    // A failed check only leaves the line out: the core still refuses a rule for such a call.
    retry: false,
  });
  // A call that sends or deletes says so plainly, and while the dot's "Always ask before
  // sending or deleting" is on it always asks: the core offers no rule for it then.
  const guard = useQuery({
    queryKey: keys.approvalGuard(approval.id),
    queryFn: () => commands.approvalGuard(approval.id),
    // A failed check only leaves the sentence out: the core still decides what asks.
    retry: false,
  });
  // A call the workspace's guardrails protect: on a Member's dot this PC can't approve it, and
  // on an Admin's it always asks (guardrails spec section 4.2). No rule is saved with it either
  // way.
  const guardrail = useQuery({
    queryKey: keys.approvalGuardrail(approval.id),
    queryFn: () => commands.approvalGuardrail(approval.id),
    // A failed check only leaves the line out: the core still refuses what it must.
    retry: false,
  }).data;
  const needsAdmin = guardrail?.needs_admin ? guardrail.text : undefined;
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
  const refusal = useRunRefusal(approval.run_id, "answer");
  const decided = decide.isPending || decide.isSuccess || refusal !== undefined;
  // Approving is off for a call that needs an Admin; Deny stays.
  const approveOff = decided || needsAdmin !== undefined;
  const approveWhy = refusal ?? needsAdmin;
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
      {guard.data && <GuardSentence guard={guard.data} dotName={dotName} />}
      {guardrail && (
        <div aria-label="Guardrails" className="flex items-start gap-1.5 text-xs">
          <ShieldAlert aria-hidden className="mt-0.5 size-3.5 shrink-0 text-needs" />
          <p className="flex flex-wrap items-center gap-1.5">
            {guardrail.needs_admin && <Badge variant="outline">Needs an Admin</Badge>}
            <span>{guardrail.text}</span>
          </p>
        </div>
      )}
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
          disabled={approveOff}
          title={approveWhy}
          onClick={onSingleClick(() => decide.mutate({ approved: true, allowRule: null }))}
        >
          Approve
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={decided}
          title={refusal}
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
            disabled={approveOff}
            title={approveWhy}
            onClick={() => setChoosing((open) => !open)}
          >
            Approve and always allow
          </Button>
        )}
      </div>
      {privateCall.data === true && (
        <p className="text-xs text-muted-foreground">
          This touches a private folder, so it always asks.
        </p>
      )}
      {guard.data?.always_asks && (
        <p className="text-xs text-muted-foreground">
          {guard.data.kind === "unclear"
            ? "Dots can't tell whether this sends or deletes, so it always asks"
            : "This sends or deletes, so it always asks"}{" "}
          while "Always ask before sending or deleting" is on for {dotName ?? "this dot"}. You can
          switch that off in the dot's permissions.
        </p>
      )}
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
              disabled={approveOff}
              title={approveWhy}
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

/// "Scout wants to send an email with Gmail (send_email)": what a guarded call does, in words.
/// The exact command or arguments are in the card below it.
function GuardSentence({ guard, dotName }: { guard: ApprovalGuard; dotName?: string }) {
  const Icon = guard.kind === "send" ? Send : guard.kind === "delete" ? Trash2 : CircleHelp;
  return (
    <p className="flex items-start gap-1.5" aria-label="What this does">
      <Icon aria-hidden className="mt-1 size-3.5 shrink-0 text-needs" />
      <span>
        <span className="font-semibold">{dotName ?? "The dot"}</span> wants to {guard.summary}
      </span>
    </p>
  );
}
