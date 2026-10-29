import { useMutation, useQueryClient } from "@tanstack/react-query";
import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Message } from "@/api/types";
import { Button } from "@/components/ui/button";
import { orgRemovedText, useDotOrg } from "@/lib/dot-org";
import {
  defaultTarget,
  isActive,
  lastRun,
  topicTitle,
  type Topic,
} from "@/lib/home-topics";
import { isOpenQuestion } from "@/lib/messages";
import { notifyError } from "@/lib/notify";
import { invalidate } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";
import { Composer } from "@/screens/home/Composer";

/// Which topic the next message goes to: the default, one the person chose with Reply, or a
/// new topic (chat-first home spec, section 5).
export type ReplyPick = { kind: "default" } | { kind: "topic"; rootId: string } | { kind: "new" };

/// The topic `pick` names among `shown`; undefined for a new topic. By default an older topic
/// shown alone (`olderRoot`), else the current chat topic. A chosen topic that is no longer
/// shown falls back to the default.
export function replyTarget(
  pick: ReplyPick,
  shown: Topic[],
  olderRoot?: string,
): Topic | undefined {
  if (pick.kind === "new") return undefined;
  const chosen = pick.kind === "topic" ? shown.find((t) => t.rootId === pick.rootId) : undefined;
  const older = olderRoot !== undefined ? shown.find((t) => t.rootId === olderRoot) : undefined;
  return chosen ?? older ?? defaultTarget(shown);
}

/// Below the conversation, in its centred column: why the target cannot be continued, the open
/// question, the reply target, and the composer. A message continues the target's last run, or starts a new chat
/// topic without one; an answer to the open question goes through `message_reply`.
export function ReplyArea(props: {
  dotId: string;
  dotName: string;
  /// The dot's org, null for Personal. A dot whose org is not joined can't run, so nothing is
  /// sent: the core would refuse the run at its start anyway.
  orgId: string | null;
  shown: Topic[];
  checkIns: Message[];
  pick: ReplyPick;
  onPick: (pick: ReplyPick) => void;
  /// The older topic shown alone, if it is one.
  olderRoot?: string;
}) {
  const { dotId, dotName, shown, pick, onPick, olderRoot } = props;
  const qc = useQueryClient();
  const navigate = useNavigate();
  const target = replyTarget(pick, shown, olderRoot);
  const { removed } = useDotOrg(props.orgId);
  const blocked = removed && props.orgId !== null ? orgRemovedText(props.orgId) : undefined;
  const busy = target?.runs.some(isActive) ?? false;
  const approving = target?.runs.some((run) => run.status === "awaiting_approval") ?? false;
  // A reply resumes the last run's session. A run that ended before its engine had one (a
  // failed start, a cancel while queued) leaves nothing to resume: the core refuses it.
  const closed = target !== undefined && !busy && lastRun(target).session_id === null;
  const runIds = new Set(shown.flatMap((topic) => topic.runs.map((run) => run.id)));
  const open = props.checkIns.filter(
    (m) => m.run_id !== null && runIds.has(m.run_id) && isOpenQuestion(m),
  );
  const question = open.at(-1);
  const questionBusy =
    question !== undefined &&
    shown.some((t) => t.runs.some((r) => r.id === question.run_id) && t.runs.some(isActive));
  // "Reply to question" pressed: the composer sends the answer to the open question.
  const [replying, setReplying] = useState(false);
  const answering = replying && question !== undefined;
  // What a send waits for before it is done: the dot's runs and the threads, loaded again. Until
  // then the screen still ends at the old run, and a second message would continue from it.
  const reload = () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: ["runs"] }),
      qc.invalidateQueries({ queryKey: ["thread"] }),
    ]);
  // An answer or a message that went to another topic than the older one shown is seen in the
  // conversation.
  const follow = (rootId: string) => {
    if (olderRoot !== undefined && rootId !== olderRoot) void navigate(`/home/${dotId}`);
  };
  const send = useMutation({
    mutationFn: (message: string) =>
      commands.chat(dotId, message, target ? lastRun(target).id : null),
    onSuccess: (run) => {
      onPick({ kind: "default" });
      follow(run.root_run_id);
      return reload();
    },
    // The core refuses a message when the topic it holds is ahead of this one. So the topics
    // are loaded again after any refusal, and the composer follows what they bring.
    onError: (error) => {
      notifyError(error);
      return reload();
    },
  });
  const answer = useMutation({
    mutationFn: (text: string) => commands.messageReply(question!.id, text),
    onSuccess: (run) => {
      setReplying(false);
      invalidate(qc, ["messages"]);
      follow(run.root_run_id);
      return reload();
    },
    onError: (error) => {
      notifyError(error);
      invalidate(qc, ["messages"]);
      return reload();
    },
  });
  // Opening the conversation marks its open questions read; they stay open until answered.
  const unread = open
    .filter((m) => m.read_at === null)
    .map((m) => m.id)
    .join(" ");
  useEffect(() => {
    if (unread === "") return;
    for (const id of unread.split(" ")) {
      void commands.messageMarkRead(id).then(
        () => invalidate(qc, ["messages"]),
        () => {},
      );
    }
  }, [unread, qc]);

  const chip =
    target === undefined
      ? "New topic"
      : olderRoot === undefined && target.kind === "chat"
        ? "Replying to: current topic"
        : `Replying to: ${topicTitle(target)}`;
  const hint = blocked
    ? "org removed"
    : answering
      ? "Your answer to the question"
      : busy
        ? approving
          ? "Waiting for your approval above"
          : `${dotName} is working on this…`
        : closed
          ? "This topic cannot be continued"
          : `Message ${dotName}…`;
  return (
    <div className="mx-auto flex w-full max-w-[720px] flex-col gap-2 px-4 pb-4">
      {blocked && (
        <p role="status" className="px-1 text-destructive">
          org removed: {blocked}
        </p>
      )}
      {closed && (
        <p className="flex items-center gap-2 px-1 text-muted-foreground">
          <span className="flex-1">
            This topic cannot be continued: its last run ended before the dot had a session to
            resume.
          </span>
          <Button size="sm" variant="outline" onClick={() => onPick({ kind: "new" })}>
            New topic
          </Button>
        </p>
      )}
      {question && (
        <div
          role="region"
          aria-label="Open question"
          className="flex flex-wrap items-center gap-2 rounded-xl border bg-card px-3 py-2"
        >
          <p className="min-w-0 flex-1 whitespace-pre-wrap">
            {dotName} asked: {question.text}
          </p>
          {question.choices?.map((choice) => (
            <Button
              key={choice}
              size="sm"
              variant="outline"
              disabled={questionBusy || answer.isPending || blocked !== undefined}
              onClick={onSingleClick(() => answer.mutate(choice))}
            >
              {choice}
            </Button>
          ))}
          <Button
            size="sm"
            variant={answering ? "default" : "outline"}
            aria-pressed={answering}
            disabled={questionBusy || blocked !== undefined}
            onClick={() => setReplying(!replying)}
          >
            Reply to question
          </Button>
        </div>
      )}
      <div
        role="group"
        aria-label="Reply target"
        aria-live="polite"
        className="flex items-center gap-1 px-1 text-xs text-muted-foreground"
      >
        <span>{chip}</span>
        {pick.kind !== "default" && (
          <Button
            size="icon-xs"
            variant="ghost"
            aria-label="Clear the reply target"
            title="Clear the reply target"
            onClick={() => onPick({ kind: "default" })}
          >
            <X aria-hidden />
          </Button>
        )}
      </div>
      <Composer
        disabled={
          blocked !== undefined ||
          (answering ? questionBusy || answer.isPending : busy || send.isPending || closed)
        }
        hint={hint}
        onSend={(message) =>
          answering ? answer.mutateAsync(message) : send.mutateAsync(message)
        }
      />
    </div>
  );
}
