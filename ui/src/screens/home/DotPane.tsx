import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { dotStatus } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { DotHeader } from "@/screens/home/DotHeader";
import { rememberDot } from "@/screens/home/last-dot";

/// One dot's side of the home: its header and its conversation. Mounted per dot.
export function DotPane({ row }: { row: DotOverview; rootRunId?: string }) {
  const dot = row.dot;
  const allMessages =
    useQuery({ queryKey: keys.messages(null), queryFn: () => commands.messagesList(null) }).data ??
    [];
  useEffect(() => rememberDot(dot.id), [dot.id]);
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <DotHeader name={dot.name} status={dotStatus(row, allMessages)} />
      <section aria-label="Conversation" className="min-h-0 flex-1 overflow-auto p-4" />
    </div>
  );
}
