import { cn } from "cn";

import { STATUS_LABEL, type DotStatus } from "@/lib/dot-status";

const LOOK: Record<DotStatus, string> = {
  waiting: "bg-needs",
  held: "border-2 border-needs bg-transparent",
  running: "animate-pulse bg-primary motion-reduce:animate-none",
  attention: "bg-destructive",
  off: "border border-muted-foreground bg-transparent",
  idle: "bg-muted-foreground",
};

/// A dot's status as a coloured dot, named for assistive technology and in its tooltip.
export function StatusMark({ status }: { status: DotStatus }) {
  return (
    <span
      role="img"
      data-status={status}
      aria-label={STATUS_LABEL[status]}
      title={STATUS_LABEL[status]}
      className={cn("inline-block size-2.5 shrink-0 rounded-full", LOOK[status])}
    />
  );
}
