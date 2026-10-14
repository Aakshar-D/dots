import { cn } from "cn";

import type { Approval } from "@/api/types";
import { APPLY_LABEL, needsAttention } from "@/lib/approvals";
import { toolPreview } from "@/lib/transcript";

/// The approved calls of a thread that Dots performs itself, each with what became of it. The
/// transcript shows a call that ran, with its output. This list also holds the calls that did
/// not run or whose outcome is unknown, which the transcript cannot show.
export function AppliedCalls({ approvals }: { approvals: Approval[] }) {
  const calls = approvals.filter((a) => a.apply_state !== null);
  if (calls.length === 0) return null;
  return (
    <ul aria-label="Approved calls" className="flex flex-col gap-1">
      {calls.map((approval) => {
        const state = approval.apply_state!;
        return (
          <li
            key={approval.id}
            className="flex items-baseline gap-2 rounded-md border px-3 py-2 text-xs"
          >
            <span className="font-semibold">{approval.tool}</span>
            <span className="min-w-0 flex-1 truncate font-mono">
              {toolPreview(approval.input)}
            </span>
            <span
              className={cn(needsAttention(state) ? "text-destructive" : "text-muted-foreground")}
            >
              {APPLY_LABEL[state]}
            </span>
          </li>
        );
      })}
    </ul>
  );
}
