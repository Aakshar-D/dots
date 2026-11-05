import type { ReactNode } from "react";

import type { Dot } from "@/api/types";
import type { DotStatus } from "@/lib/dot-status";
import { DotAvatar } from "@/components/DotAvatar";
import { DotMenu } from "@/screens/home/DotMenu";
import { StatusMark } from "@/screens/home/StatusMark";

/// The top of a dot's conversation, slim and with no bar (conversational chat spec section 5):
/// its name, which opens its menu, and its status mark, then the actions it is given.
export function DotHeader(props: {
  dot: Dot;
  status: DotStatus;
  /// When its schedule last skipped a slot because the dot was busy; "" shows nothing.
  skipLine?: string;
  children?: ReactNode;
}) {
  return (
    // Wraps its actions onto a second line in a narrow window rather than scrolling sideways.
    <header className="flex flex-wrap items-center gap-x-2 gap-y-1 px-4 py-2">
      <DotAvatar seed={props.dot.id} size="md" />
      <h1 className="flex min-w-0">
        <DotMenu dot={props.dot} />
      </h1>
      <StatusMark status={props.status} />
      {props.skipLine && <span className="text-xs text-muted-foreground">{props.skipLine}</span>}
      <span className="flex-1" />
      {props.children}
    </header>
  );
}
