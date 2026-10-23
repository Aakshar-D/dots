import type { RunStatus } from "@/api/types";
import { Badge } from "@/components/ui/badge";

const LOOK: Record<RunStatus, { label: string; className: string }> = {
  queued: { label: "Queued", className: "bg-muted text-muted-foreground" },
  running: { label: "Running", className: "border-primary/40 bg-transparent text-primary" },
  awaiting_approval: {
    label: "Needs approval",
    className: "border-needs-border bg-transparent text-needs",
  },
  succeeded: { label: "Succeeded", className: "border-success/40 bg-transparent text-success" },
  failed: { label: "Failed", className: "border-destructive/40 bg-transparent text-destructive" },
  cancelled: { label: "Cancelled", className: "bg-muted text-muted-foreground" },
};

export function statusLabel(status: RunStatus): string {
  return LOOK[status].label;
}

export function StatusBadge({ status }: { status: RunStatus }) {
  const { label, className } = LOOK[status];
  return (
    <Badge variant="secondary" className={className}>
      {label}
    </Badge>
  );
}
