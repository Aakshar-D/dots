import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { commands } from "@/api/commands";
import type { InstructionProposal, Message, ScheduleSkip } from "@/api/types";
import { AppReviews } from "@/components/AppReviewCard";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import type { Topic } from "@/lib/home-topics";
import { keys } from "@/lib/query";
import { useStickToBottom } from "@/lib/use-stick-to-bottom";
import { RunCard } from "@/screens/home/RunCard";
import { ScheduledCards } from "@/screens/home/ScheduledCard";
import { SkipLine } from "@/screens/home/SkipLine";
import { TopicRuns } from "@/screens/home/TopicRuns";

type Entry = { topic: Topic } | { skip: ScheduleSkip };

/// The topics with the skipped slots among them by time (a skip stands after the last topic that
/// began before it). `skips` is newest first. With topics shown, a skip from before the first
/// of them is left out: it belongs to the history.
function mergeSkips(topics: Topic[], skips: ScheduleSkip[]): Entry[] {
  const entries: Entry[] = topics.map((topic) => ({ topic }));
  const start = (topic: Topic) => topic.runs[0].queued_at;
  for (const skip of [...skips].reverse()) {
    if (topics.length > 0 && skip.slot_at.localeCompare(start(topics[0])) < 0) continue;
    let at = 0;
    entries.forEach((entry, i) => {
      if ("topic" in entry && start(entry.topic).localeCompare(skip.slot_at) <= 0) at = i + 1;
    });
    // After the skips already placed there, so equal times keep their order.
    while (at < entries.length && "skip" in entries[at]) at++;
    entries.splice(at, 0, { skip });
  }
  return entries;
}

/// The topics shown, in order: a chat topic as a transcript, a run topic as a card with the
/// runs that continued it under it. It stays at the bottom as things arrive unless the person
/// scrolled up; then "New activity" takes them down (chat-first home spec, section 4).
export function Timeline(props: {
  topics: Topic[];
  checkIns: Message[];
  dotName: string;
  /// The dot whose skipped schedule slots stand among the topics; none shows no skips (an
  /// older topic on its own).
  skipsDotId?: string;
  /// The instructions the dot proposed that wait for the person.
  proposals: InstructionProposal[];
  /// The dot whose scheduled messages wait as cards after the topics; with `root` (an older
  /// topic on its own), only those of that topic.
  scheduledFor?: { dotId: string; root?: string };
  /// Reply on a run card: makes its topic the reply target.
  onReply: (topic: Topic) => void;
  /// Shown above the topics: a failed load, or what to do when there is none.
  children?: ReactNode;
}) {
  const { topics, checkIns, dotName } = props;
  const { ref, onScroll, behind, jump } = useStickToBottom<HTMLElement>();
  // The cards of apps waiting for review read this list (the same query); a failure to read it
  // is shown here once, not under every run.
  const dotId = topics[0]?.runs[0]?.dot_id ?? null;
  const proposals = useQuery({
    queryKey: keys.appProposals(dotId),
    queryFn: () => commands.appProposalsList(dotId),
    enabled: dotId !== null,
  });
  // The slots skipped because the dot was busy: a query of their own, never messages, so they
  // cannot change what is unread. They stand among the topics by time.
  const skipsOf = props.skipsDotId ?? null;
  const skipsQuery = useQuery({
    queryKey: keys.scheduleSkips(skipsOf ?? ""),
    queryFn: () => commands.scheduleSkips(skipsOf!),
    enabled: skipsOf !== null,
  });
  const skips = skipsQuery.data ?? [];
  const entries = mergeSkips(topics, skips);
  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <section
        ref={ref}
        onScroll={onScroll}
        aria-label="Conversation"
        className="flex min-h-0 flex-1 flex-col overflow-auto"
      >
        {/* The scrolling area keeps the pane's width, so its scrollbar is at the pane's edge;
            what it shows is one centred column (conversational chat spec section 5). */}
        <div className="mx-auto flex w-full max-w-[720px] flex-col gap-6 px-4 py-4">
          {props.children}
          {proposals.isError && <ErrorNote error={proposals.error} />}
          {skipsQuery.isError && <ErrorNote error={skipsQuery.error} />}
          {entries.map((entry) => {
            if ("skip" in entry) return <SkipLine key={entry.skip.id} skip={entry.skip} />;
            const topic = entry.topic;
            return topic.kind === "chat" ? (
              <TopicRuns
                key={topic.rootId}
                runs={topic.runs}
                checkIns={checkIns}
                dotName={dotName}
                proposals={props.proposals}
              />
            ) : (
              <div key={topic.rootId} className="flex flex-col gap-4">
                <RunCard
                  topic={topic}
                  checkIns={checkIns}
                  dotName={dotName}
                  onReply={() => props.onReply(topic)}
                />
                {/* An app the run sent for review waits under its card. */}
                <AppReviews runs={topic.runs.slice(0, 1)} dotName={dotName} />
                {topic.runs.length > 1 && (
                  <TopicRuns runs={topic.runs.slice(1)} checkIns={checkIns} dotName={dotName} />
                )}
              </div>
            );
          })}
          {props.scheduledFor && (
            <ScheduledCards
              dotId={props.scheduledFor.dotId}
              dotName={dotName}
              topics={topics}
              root={props.scheduledFor.root}
            />
          )}
        </div>
      </section>
      {behind && (
        <Button
          size="sm"
          className="absolute bottom-3 left-1/2 -translate-x-1/2 shadow"
          onClick={jump}
        >
          New activity ↓
        </Button>
      )}
    </div>
  );
}
