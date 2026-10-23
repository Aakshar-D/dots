import { cn } from "cn";

import type { DotOverview } from "@/api/types";

/// How many things a dot has for the person (needs-you spec section 5): its unread count, in
/// the waiting colour while something needs them, else muted. Assistive technology reads
/// "3 unread". Nothing at 0.
export function UnreadBadge({ row }: { row: DotOverview }) {
  if (row.unread <= 0) return null;
  const waiting = row.needs_you > 0;
  return (
    <span
      data-tone={waiting ? "waiting" : "muted"}
      className={cn(
        "ml-auto inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full px-1 text-[10px] font-medium",
        waiting ? "bg-amber-400 text-amber-950" : "bg-muted text-muted-foreground",
      )}
    >
      <span aria-hidden>{row.unread}</span>
      <span className="sr-only">{row.unread} unread</span>
    </span>
  );
}
