import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Message } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// A dot's question or note in the Inbox (check-ins spec section 9). A question offers its
/// choices, a reply box and Dismiss; a note offers Mark read. A question shown here counts as
/// read, and stays open until it is answered or dismissed.
export function MessageCard({ message }: { message: Message }) {
  const qc = useQueryClient();
  const [text, setText] = useState("");
  // What a change shows up in: the messages, and the thread and runs an answer adds a run to.
  const reload = () => {
    for (const queryKey of [["messages"], ["runs"], ["thread"]]) invalidate(qc, queryKey);
  };
  const reply = useMutation({
    mutationFn: (answer: string) => commands.messageReply(message.id, answer),
    onSuccess: () => {
      setText("");
      toast.success(`Answer sent to ${message.dot_name}`);
    },
    onError: notifyError,
    onSettled: reload,
  });
  const dismiss = useMutation({
    mutationFn: () => commands.messageDismiss(message.id),
    onError: notifyError,
    onSettled: reload,
  });
  const markRead = useMutation({
    mutationFn: () => commands.messageMarkRead(message.id),
    onError: notifyError,
    onSettled: reload,
  });
  const question = message.kind === "question";
  const unreadQuestion = question && message.read_at === null;
  const markReadNow = markRead.mutate;
  useEffect(() => {
    if (unreadQuestion) markReadNow();
  }, [unreadQuestion, markReadNow]);
  // One answer per question: after a success the card waits for the list to drop it.
  const answering = reply.isPending || reply.isSuccess;
  return (
    <li
      aria-label={`${question ? "Question" : "Note"} from ${message.dot_name}`}
      className="flex flex-col gap-2 rounded-lg border p-3"
    >
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <Link to={`/dots/${message.dot_id}`} className="font-medium text-foreground hover:underline">
          {message.dot_name}
        </Link>
        <span>
          {question ? "asks" : "says"} · {relativeTime(message.created_at)}
        </span>
        {message.run_id && (
          <Link to={`/runs/${message.run_id}`} className="hover:underline">
            Open the run
          </Link>
        )}
        {/* The old chat address of the run opens its topic, in the conversation or alone. */}
        <Link
          to={
            message.run_id
              ? `/chat/${message.dot_id}/${message.run_id}`
              : `/home/${message.dot_id}`
          }
          className="hover:underline"
        >
          Open conversation
        </Link>
      </p>
      <p className="whitespace-pre-wrap">{message.text}</p>
      {question ? (
        <>
          {message.choices && message.choices.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {message.choices.map((choice) => (
                <Button
                  key={choice}
                  size="sm"
                  variant="outline"
                  disabled={answering}
                  onClick={onSingleClick(() => reply.mutate(choice))}
                >
                  {choice}
                </Button>
              ))}
            </div>
          )}
          <div className="flex items-end gap-2">
            <label htmlFor={`answer-${message.id}`} className="sr-only">
              Answer to {message.dot_name}: {message.text}
            </label>
            <Textarea
              id={`answer-${message.id}`}
              rows={2}
              placeholder="Type an answer"
              value={text}
              disabled={answering}
              onChange={(e) => setText(e.target.value)}
            />
            <Button
              size="sm"
              disabled={answering || text.trim() === ""}
              onClick={onSingleClick(() => reply.mutate(text.trim()))}
            >
              Reply
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={answering || dismiss.isPending}
              onClick={onSingleClick(() => dismiss.mutate())}
            >
              Dismiss
            </Button>
          </div>
        </>
      ) : (
        <div>
          <Button
            size="sm"
            variant="outline"
            disabled={markRead.isPending}
            onClick={onSingleClick(() => markRead.mutate())}
          >
            Mark read
          </Button>
        </div>
      )}
    </li>
  );
}
