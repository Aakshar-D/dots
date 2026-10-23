import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

/// A list with nothing in it (deep-teal spec section 4): an icon and one muted line.
export function EmptyState({ icon: Icon, children }: { icon: LucideIcon; children: ReactNode }) {
  return (
    <div
      data-slot="empty-state"
      className="flex items-center gap-3 rounded-lg border border-dashed p-4 text-muted-foreground"
    >
      <Icon aria-hidden className="size-5 shrink-0" />
      <p>{children}</p>
    </div>
  );
}
