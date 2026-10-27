import type { ReactNode } from "react";

import { STATUS_LABEL, type DotStatus } from "@/lib/dot-status";
import { StatusMark } from "@/screens/home/StatusMark";

/// The top of a dot's conversation: its name and status, then the actions it is given.
export function DotHeader(props: { name: string; status: DotStatus; children?: ReactNode }) {
  return (
    // Wraps its actions onto a second line in a narrow window rather than scrolling sideways.
    <header className="flex flex-wrap items-center gap-x-2 gap-y-1 border-b px-4 py-2">
      <h1 className="min-w-0 truncate text-base font-semibold">{props.name}</h1>
      <StatusMark status={props.status} />
      <span className="flex-1 text-xs text-muted-foreground">{STATUS_LABEL[props.status]}</span>
      {props.children}
    </header>
  );
}
