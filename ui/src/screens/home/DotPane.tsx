import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, Navigate } from "react-router";

import { CommandError, commands } from "@/api/commands";
import type { DotOverview } from "@/api/types";
import { LoadErrors } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { dotStatus } from "@/lib/dot-status";
import { keys } from "@/lib/query";
import { DotHeader } from "@/screens/home/DotHeader";
import { HistoryMenu } from "@/screens/home/HistoryMenu";
import { rememberDot } from "@/screens/home/last-dot";
import { ReplyArea, type ReplyPick } from "@/screens/home/ReplyArea";
import { Timeline } from "@/screens/home/Timeline";
import { useConversation } from "@/screens/home/use-conversation";
import { useViewing } from "@/screens/home/use-viewing";

/// One dot's side of the home: its header and its conversation, or one older topic of it
/// (`rootRunId`). Mounted per dot and topic.
export function DotPane({ row, rootRunId }: { row: DotOverview; rootRunId?: string }) {
  const dot = row.dot;
  const allMessages =
    useQuery({ queryKey: keys.messages(null), queryFn: () => commands.messagesList(null) }).data ??
    [];
  // The instructions the dot proposed that wait for the person (conversational chat spec
  // section 4), each on a card under the run that proposed it.
  const proposals = (
    useQuery({
      queryKey: keys.instructionProposals(dot.id),
      queryFn: () => commands.instructionProposalsList(dot.id),
    }).data ?? []
  ).filter((p) => p.status === "pending");
  const held =
    useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data?.held_run_ids ?? [];
  useEffect(() => rememberDot(dot.id), [dot.id]);
  useViewing(dot.id);
  const view = useConversation(dot.id, rootRunId);
  // An older topic whose run is gone (deleted, or a stale link) leaves nothing to show.
  const older = rootRunId !== undefined ? view.threads[0] : undefined;
  const gone =
    older !== undefined &&
    ((older.error instanceof CommandError && older.error.kind === "not_found") ||
      (older.isSuccess && older.data.length === 0));
  const [pick, setPick] = useState<ReplyPick>({ kind: "default" });
  // The current conversation, loaded and with nothing in it, greets like a new chat
  // (conversational chat spec section 5). A failed load shows its errors instead.
  const greeting =
    rootRunId === undefined && !view.pending && view.list.isSuccess && view.shown.length === 0;
  if (gone) return <Navigate to={`/home/${dot.id}`} replace />;
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <DotHeader dot={dot} status={dotStatus(row, allMessages, held)}>
        <HistoryMenu dotId={dot.id} topics={view.older} />
        <Button size="sm" variant="ghost" onClick={() => setPick({ kind: "new" })}>
          New topic
        </Button>
      </DotHeader>
      {rootRunId !== undefined && (
        <p className="mx-auto w-full max-w-[720px] px-4 py-1.5 text-xs text-muted-foreground">
          Older topic ·{" "}
          <Link to={`/home/${dot.id}`} className="text-foreground underline">
            Back to the current conversation
          </Link>
        </p>
      )}
      {greeting ? (
        // Above the composer and pushed down by as much space as the spacer under it, so that
        // the greeting and the composer sit together in the middle of the pane.
        <div className="mx-auto flex w-full max-w-[720px] flex-1 flex-col items-center justify-end gap-1 px-4 pb-6 text-center">
          <h2 className="text-2xl font-semibold">{dot.name}</h2>
          <p className="text-muted-foreground">Ask {dot.name} something</p>
        </div>
      ) : (
        <Timeline
          topics={view.pending ? [] : view.shown}
          checkIns={view.checkIns}
          dotName={dot.name}
          proposals={proposals}
          onReply={(topic) => setPick({ kind: "topic", rootId: topic.rootId })}
        >
          <LoadErrors queries={[view.list, ...view.threads]} />
        </Timeline>
      )}
      {/* The reply area keeps its place in the tree when the greeting gives way to the
          conversation, so a send on its way is not cut off. */}
      {!view.pending && (
        <ReplyArea
          dotId={dot.id}
          dotName={dot.name}
          orgId={dot.org_id}
          shown={view.shown}
          checkIns={view.checkIns}
          pick={pick}
          onPick={setPick}
          olderRoot={rootRunId}
        />
      )}
      {greeting && <div aria-hidden className="flex-1" />}
    </div>
  );
}
