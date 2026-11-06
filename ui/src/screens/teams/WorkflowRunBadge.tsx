import type { WaitingOn, WorkflowRunStatus } from "@/api/types";
import { Badge } from "@/components/ui/badge";

import { workflowRunStatusLabel } from "./format";

const LOOK: Record<WorkflowRunStatus, string> = {
  running: "border-primary/40 bg-transparent text-primary",
  // A waiting run waits for the person: the needs colours (deep-teal spec section 4).
  waiting: "border-needs-border bg-transparent text-needs",
  succeeded: "border-success/40 bg-transparent text-success",
  failed: "border-destructive/40 bg-transparent text-destructive",
  cancelled: "bg-muted text-muted-foreground",
};

/// A workflow run's status as a badge, saying what a waiting run waits for when it is known.
export function WorkflowRunBadge(props: {
  status: WorkflowRunStatus;
  waitingOn?: WaitingOn | null;
}) {
  return (
    <Badge variant="secondary" data-status={props.status} className={LOOK[props.status]}>
      {workflowRunStatusLabel({ status: props.status, waiting_on: props.waitingOn })}
    </Badge>
  );
}

/// The "Test" label of a test run (teams spec section 5.1): its deploys pushed nothing.
export function TestBadge() {
  return (
    <Badge variant="outline" title="A test run: its deploys push nothing">
      Test
    </Badge>
  );
}
