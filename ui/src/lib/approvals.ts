import type { ApplyState, Approval, PendingApproval } from "@/api/types";

/// The pending approvals of one run, which the Inbox shows together.
export type RunApprovals = {
  runId: string;
  dotId: string;
  dotName: string;
  approvals: PendingApproval[];
};

/// Groups pending approvals by their run. A group stands where its first approval stood, and
/// the approvals inside a group keep the order the core gave them, the order they were asked.
export function byRun(approvals: PendingApproval[]): RunApprovals[] {
  const groups = new Map<string, RunApprovals>();
  for (const approval of approvals) {
    const group = groups.get(approval.run_id);
    if (group) {
      group.approvals.push(approval);
    } else {
      groups.set(approval.run_id, {
        runId: approval.run_id,
        dotId: approval.dot_id,
        dotName: approval.dot_name,
        approvals: [approval],
      });
    }
  }
  return [...groups.values()];
}

/// What became of a call that Dots performs itself, in words.
export const APPLY_LABEL: Record<ApplyState, string> = {
  queued: "Waiting to be performed",
  applying: "Being performed",
  applied: "Performed by Dots",
  failed: "Performed; the tool reported an error",
  unknown: "Outcome unknown: Dots stopped while this ran",
  skipped: "Not performed",
};

/// The outcomes a person should look at: the call did not do what was approved, or nobody
/// knows whether it did.
export function needsAttention(state: ApplyState): boolean {
  return state === "failed" || state === "unknown";
}

/// What a parked approval's card says about approving it. A call that Dots performs itself is
/// run exactly as shown; otherwise the model is asked to issue it again in a new run.
export function parkedNote(approval: Approval): string {
  return approval.self_apply
    ? "The run stopped waiting and is parked. If you approve, Dots performs exactly this call and then continues the run."
    : "The run stopped waiting and is parked. Approving continues it in a new run.";
}
