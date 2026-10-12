import type { RunStatus } from "@/api/types";
import { Badge } from "@/components/ui/badge";

const LOOK: Record<RunStatus, { label: string; className: string }> = {
  queued: { label: "Queued", className: "bg-muted text-muted-foreground" },
  running: { label: "Running", className: "bg-sky-500/20 text-sky-300" },
  awaiting_approval: { label: "Needs approval", className: "bg-amber-500/20 text-amber-300" },
  succeeded: { label: "Succeeded", className: "bg-emerald-500/20 text-emerald-300" },
  failed: { label: "Failed", className: "bg-red-500/20 text-red-300" },
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
