import { useMutation, useQueries, useQueryClient } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { InstructionProposal, Message, Run, RunEventRecord } from "@/api/types";
import { AppliedCalls } from "@/components/AppliedCalls";
import { AppReviews } from "@/components/AppReviewCard";
import { ApprovalCard } from "@/components/ApprovalCard";
import { LoadErrors } from "@/components/ErrorNote";
import { InstructionsCard } from "@/components/InstructionsCard";
import { Failure, Transcript } from "@/components/Transcript";
import { Button } from "@/components/ui/button";
import { isActive } from "@/lib/home-topics";
import { errorMessage } from "@/lib/notify";
import { eventsQuery, invalidate, keys } from "@/lib/query";

import { QuietCard } from "./QuietCard";

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
  // A proposal can't be decided while the dot still works on its topic.
  const busy = runs.some(isActive);
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
      {latest && <QuietCard run={latest} />}
      <AppReviews runs={runs} dotName={dotName} />
      {proposals.map((proposal) => (
        <InstructionsCard key={proposal.id} proposal={proposal} dotName={dotName} busy={busy} />
      ))}
      {failure && <Failure>{failure}</Failure>}
      {latest?.status === "failed" &&
        latest.trigger === "chat" &&
        latest.workflow_step_run_id === null && <SendAgain run={latest} />}
    </div>
  );
}

/// Send again (crash resend spec section 3.6): the failed message of the topic's last run,
/// queued once more in the topic. The core refuses when the topic has moved on; its text shows.
function SendAgain({ run }: { run: Run }) {
  const qc = useQueryClient();
  const send = useMutation({
    mutationFn: commands.resendRun,
    onSuccess: () => {
      invalidate(qc, ["thread"]);
      invalidate(qc, ["runs"]);
      invalidate(qc, keys.overview);
    },
  });
  return (
    <div className="flex flex-col gap-1">
      <Button
        size="sm"
        variant="outline"
        className="self-start"
        disabled={send.isPending}
        onClick={() => send.mutate(run.id)}
      >
        Send again
      </Button>
      {send.isError && (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage(send.error)}
        </p>
      )}
    </div>
  );
}
