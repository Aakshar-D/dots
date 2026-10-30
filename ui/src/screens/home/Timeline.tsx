import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { commands } from "@/api/commands";
import type { InstructionProposal, Message } from "@/api/types";
import { AppReviews } from "@/components/AppReviewCard";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import type { Topic } from "@/lib/home-topics";
import { keys } from "@/lib/query";
import { useStickToBottom } from "@/lib/use-stick-to-bottom";
import { RunCard } from "@/screens/home/RunCard";
import { TopicRuns } from "@/screens/home/TopicRuns";

/// The topics shown, in order: a chat topic as a transcript, a run topic as a card with the
/// runs that continued it under it. It stays at the bottom as things arrive unless the person
/// scrolled up; then "New activity" takes them down (chat-first home spec, section 4).
export function Timeline(props: {
  topics: Topic[];
  checkIns: Message[];
  dotName: string;
  /// The instructions the dot proposed that wait for the person.
  proposals: InstructionProposal[];
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
          {topics.map((topic) =>
            topic.kind === "chat" ? (
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
            ),
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
