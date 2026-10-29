import { useQueries, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { InstructionProposal, Message, Run, RunEventRecord } from "@/api/types";
import { AppliedCalls } from "@/components/AppliedCalls";
import { AppReviews } from "@/components/AppReviewCard";
import { ApprovalCard } from "@/components/ApprovalCard";
import { LoadErrors } from "@/components/ErrorNote";
import { InstructionsCard } from "@/components/InstructionsCard";
import { Failure, Transcript } from "@/components/Transcript";
import { eventsQuery, keys } from "@/lib/query";

/// Runs of one topic as the chat shows a thread: each run's message, the dot's text, its tool
/// steps, check-ins and browser calls (`Transcript`), the calls Dots performed, a card per
/// approval it waits for and per app it sent for review, the instructions the dot proposed that
/// wait for the person, and why the last run failed.
export function TopicRuns(props: {
  runs: Run[];
  checkIns: Message[];
  dotName: string;
  /// The dot's waiting proposals; the topic shows those its runs made.
  proposals?: InstructionProposal[];
}) {
  const { runs, dotName } = props;
  const qc = useQueryClient();
  const events = useQueries({ queries: runs.map((run) => eventsQuery(qc, run.id)) });
  const approvals = useQueries({
    queries: runs.map((run) => ({
      queryKey: keys.approvals(run.id),
      queryFn: () => commands.approvalsForRun(run.id),
    })),
  });
  const eventsByRun = new Map<string, RunEventRecord[]>(
    runs.map((run, i) => [run.id, events[i]?.data ?? []]),
  );
  const asked = approvals.flatMap((q) => q.data ?? []);
  const pending = asked.filter((a) => a.status === "pending");
  const latest = runs.at(-1);
  // Why the last run failed stands below its transcript, so the transcript does not repeat it.
  const failure = latest?.status === "failed" ? latest.error : null;
  const checkIns = props.checkIns.filter((m) => runs.some((run) => run.id === m.run_id));
  const proposals = (props.proposals ?? []).filter((p) =>
    runs.some((run) => run.id === p.run_id),
  );
  return (
    <div className="flex flex-col gap-4">
      <Transcript
        runs={runs}
        events={eventsByRun}
        shownError={failure}
        messages={checkIns}
        dotName={dotName}
        variant="conversation"
      />
      <LoadErrors queries={[...events, ...approvals]} />
      <AppliedCalls approvals={asked} />
      {pending.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={dotName} />
      ))}
      <AppReviews runs={runs} dotName={dotName} />
      {proposals.map((proposal) => (
        <InstructionsCard key={proposal.id} proposal={proposal} dotName={dotName} />
      ))}
      {failure && <Failure>{failure}</Failure>}
    </div>
  );
}
