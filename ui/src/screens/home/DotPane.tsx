import { useQuery } from "@tanstack/react-query";
import { useEffect } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { LoadErrors } from "@/components/ErrorNote";
import { dotStatus } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { DotHeader } from "@/screens/home/DotHeader";
import { rememberDot } from "@/screens/home/last-dot";
import { Timeline } from "@/screens/home/Timeline";
import { useConversation } from "@/screens/home/use-conversation";

/// One dot's side of the home: its header and its conversation, or one older topic of it
/// (`rootRunId`). Mounted per dot.
export function DotPane({ row, rootRunId }: { row: DotOverview; rootRunId?: string }) {
  const dot = row.dot;
  const allMessages =
    useQuery({ queryKey: keys.messages(null), queryFn: () => commands.messagesList(null) }).data ??
    [];
  useEffect(() => rememberDot(dot.id), [dot.id]);
  const view = useConversation(dot.id, rootRunId);
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <DotHeader name={dot.name} status={dotStatus(row, allMessages)} />
      {rootRunId !== undefined && (
        <p className="border-b px-4 py-1.5 text-xs text-muted-foreground">
          Older topic ·{" "}
          <Link to={`/home/${dot.id}`} className="text-foreground underline">
            Back to the current conversation
          </Link>
        </p>
      )}
      <Timeline
        topics={view.pending ? [] : view.shown}
        checkIns={view.checkIns}
        dotName={dot.name}
      >
        <LoadErrors queries={[view.list, ...view.threads]} />
        {!view.pending && view.shown.length === 0 && view.list.isSuccess && (
          <p className="text-muted-foreground">
            Ask {dot.name} something. It answers with its own instructions, folder and
            permissions.
          </p>
        )}
      </Timeline>
    </div>
  );
}
