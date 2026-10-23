import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Globe } from "lucide-react";

import { commands } from "@/api/commands";
import type { Approval } from "@/api/types";
import { Button } from "@/components/ui/button";
import { askDetails, askParts, type BrowserAsk } from "@/lib/browser";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

type Decision = { approved: boolean; allowRule: string | null };

/// What a parked browser card says: an approval decided now does not perform the call
/// (browser spec section 3.6).
export const BROWSER_PARKED =
  "The run stopped waiting. Approving now does not perform this action: the dot is told to take a snapshot and do it again if it is still right.";

/// A browser call that waits for the person (browser spec section 3.6): what the dot wants to
/// do and where, then Allow once, Always allow on <site>, for a risky act Always allow risky
/// actions on <site>, and Deny. The rules are the ones the core offers for this call; a
/// dangerous call is offered none.
export function BrowserApprovalCard({
  approval,
  ask,
  dotName,
}: {
  approval: Approval;
  ask: BrowserAsk;
  dotName?: string;
}) {
  const qc = useQueryClient();
  const rules = useQuery({
    queryKey: keys.approvalRules(approval.id),
    queryFn: () => commands.approvalRuleCandidates(approval.id),
  });
  const offered = rules.data ?? [];
  const site = offered.find((rule) => rule.pattern.startsWith("Browser("));
  const risky = offered.find((rule) => rule.pattern.startsWith("BrowserRisky("));
  const decide = useMutation({
    mutationFn: ({ approved, allowRule }: Decision) =>
      commands.decideApproval(approval.id, approved, null, allowRule),
    onSuccess: (_answer, { allowRule }) => {
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.approvals(approval.run_id));
      // A saved rule changed the dot's policy, and no runtime event announces a dot change.
      if (allowRule !== null) invalidateDots(qc);
    },
    onError: notifyError,
  });
  // One decision per card, as on the tool card.
  const decided = decide.isPending || decide.isSuccess;
  const who = dotName ?? "The dot";
  const choose = (decision: Decision) => onSingleClick(() => decide.mutate(decision));
  return (
    <article
      aria-label={`Browser approval for ${who}`}
      className="flex flex-col gap-2 rounded-xl border border-needs-border bg-needs-surface p-3"
    >
      <p className="flex items-start gap-1.5">
        <Globe aria-hidden className="mt-1 size-3.5 shrink-0 text-needs" />
        <span>
          <span className="font-semibold text-needs">{who}</span>{" "}
          {askParts(ask).map((part, i) =>
            part.strong ? <strong key={i}>{part.text}</strong> : <span key={i}>{part.text}</span>,
          )}
        </span>
      </p>
      {askDetails(approval.tool, approval.input).length > 0 && (
        <ul aria-label="What it sends" className="flex flex-col gap-0.5 text-xs">
          {askDetails(approval.tool, approval.input).map((line, i) => (
            <li key={i}>
              <code className="break-all whitespace-pre-wrap">{line}</code>
            </li>
          ))}
        </ul>
      )}
      {ask.dangerous && (
        <p className="text-xs text-muted-foreground">No rule can allow this: it asks every time.</p>
      )}
      {approval.parked && <p className="text-xs text-muted-foreground">{BROWSER_PARKED}</p>}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={decided} onClick={choose({ approved: true, allowRule: null })}>
          Allow once
        </Button>
        {site && ask.site && (
          <Button
            size="sm"
            variant="outline"
            disabled={decided}
            onClick={choose({ approved: true, allowRule: site.pattern })}
          >
            Always allow on {ask.site}
          </Button>
        )}
        {risky && ask.site && (
          <Button
            size="sm"
            variant="outline"
            disabled={decided}
            onClick={choose({ approved: true, allowRule: risky.pattern })}
          >
            Always allow risky actions on {ask.site}
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          disabled={decided}
          onClick={choose({ approved: false, allowRule: null })}
        >
          Deny
        </Button>
      </div>
    </article>
  );
}
