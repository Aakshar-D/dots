import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { Message } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ErrorNote } from "@/components/ErrorNote";
import { StatusBadge } from "@/components/StatusBadge";
import { Button } from "@/components/ui/button";
import { oneLine, topicTitle, type Topic } from "@/lib/home-topics";
import { isOpenQuestion } from "@/lib/messages";
import { keys } from "@/lib/query";

/// A topic that a schedule, a webhook, Run now or a follow-up started, as one card: its trigger
/// and time, its status, the first line of its summary or error, Details and Reply. While its run
/// waits for an approval or has an open question the card is open and shows them; closed, it
/// loads nothing (chat-first home spec, section 4.2).
export function RunCard(props: {
  topic: Topic;
  checkIns: Message[];
  dotName: string;
  /// Makes this topic the reply target.
  onReply: () => void;
}) {
  const { topic, dotName } = props;
  const run = topic.root;
  const waiting = run.status === "awaiting_approval";
  const approvals = useQuery({
    queryKey: keys.approvals(run.id),
    queryFn: () => commands.approvalsForRun(run.id),
    enabled: waiting,
  });
  const pending = waiting ? (approvals.data ?? []).filter((a) => a.status === "pending") : [];
  const question = props.checkIns.filter((m) => m.run_id === run.id && isOpenQuestion(m)).at(-1);
  const summary = oneLine(run.summary);
  const error = summary === "" ? oneLine(run.error) : "";
  return (
    <article
      aria-label={topicTitle(topic)}
      className="flex flex-col gap-2 rounded-lg border bg-card p-3"
    >
      <div className="flex items-center gap-2">
        <span className="font-medium">{topicTitle(topic)}</span>
        <StatusBadge status={run.status} />
        <span className="flex-1" />
        <Button asChild size="xs" variant="ghost">
          <Link to={`/runs/${run.id}`}>Details</Link>
        </Button>
        <Button size="xs" variant="ghost" onClick={props.onReply}>
          Reply
        </Button>
      </div>
      {summary && <p className="text-muted-foreground">{summary}</p>}
      {error && <p className="text-destructive">{error}</p>}
      {waiting && approvals.isError && <ErrorNote error={approvals.error} />}
      {pending.map((approval) => (
        <ApprovalCard key={approval.id} approval={approval} dotName={dotName} />
      ))}
      {question && (
        <div
          role="group"
          aria-label="Question from the dot"
          className="flex flex-col gap-1 rounded-lg bg-muted px-3 py-1.5"
        >
          <p className="whitespace-pre-wrap">{question.text}</p>
          <span className="text-xs text-muted-foreground">
            Question · waiting for your answer
          </span>
        </div>
      )}
    </article>
  );
}
