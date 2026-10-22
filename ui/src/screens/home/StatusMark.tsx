import { cn } from "cn";

import { STATUS_LABEL, type DotStatus } from "@/lib/dot-status";

const LOOK: Record<DotStatus, string> = {
  waiting: "bg-amber-400",
  running: "animate-pulse bg-sky-400",
  attention: "bg-red-500",
  off: "border border-muted-foreground bg-transparent",
  idle: "bg-muted-foreground/40",
};

/// A dot's status as a coloured dot, named for assistive technology and in its tooltip.
export function StatusMark({ status }: { status: DotStatus }) {
  return (
    <span
      role="img"
      aria-label={STATUS_LABEL[status]}
      title={STATUS_LABEL[status]}
      className={cn("inline-block size-2.5 shrink-0 rounded-full", LOOK[status])}
    />
  );
}
