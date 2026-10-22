import type { ReactNode } from "react";

import { STATUS_LABEL, type DotStatus } from "@/lib/dot-status";
import { StatusMark } from "@/screens/home/StatusMark";

/// The top of a dot's conversation: its name and status, then the actions it is given.
export function DotHeader(props: { name: string; status: DotStatus; children?: ReactNode }) {
  return (
    <header className="flex items-center gap-2 border-b px-4 py-2">
      <h1 className="truncate text-base font-semibold">{props.name}</h1>
      <StatusMark status={props.status} />
      <span className="flex-1 text-xs text-muted-foreground">{STATUS_LABEL[props.status]}</span>
      {props.children}
    </header>
  );
}
