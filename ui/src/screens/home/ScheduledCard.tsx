import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Clock } from "lucide-react";
import { useId, useState } from "react";

import { commands } from "@/api/commands";
import type { ScheduledMessage } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { localDateTime, localWeekdayTime } from "@/lib/format";
import { currentTopic, topicTitle, type Topic } from "@/lib/home-topics";
import { fromPicker, toPicker } from "@/lib/later";
import { errorMessage } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { sendsIn, waitingText } from "@/lib/send-later";
import { useNow } from "@/lib/use-now";

const WEEK_MS = 7 * 86_400_000;

/// Which topic a scheduled message goes to, as its card names it: a new topic, the current
/// chat topic, another topic shown (its title), or one that is not shown.
export function scheduledTarget(
  message: ScheduledMessage,
  topics: Topic[],
  currentRoot?: string,
): string {
  if (message.root_run_id === null) return "new topic";
  if (message.root_run_id === currentRoot) return "current topic";
  const shown = topics.find((t) => t.rootId === message.root_run_id);
  return shown ? topicTitle(shown) : "an older topic";
}

/// The dot's pending scheduled messages, at the end of the conversation in due order (send
/// later spec section 3.1.11). With `root` (an older topic shown on its own), only its own.
export function ScheduledCards(props: {
  dotId: string;
  dotName: string;
  topics: Topic[];
  root?: string;
}) {
  const query = useQuery({
    queryKey: keys.scheduledMessages(props.dotId),
    queryFn: () => commands.scheduledMessages(props.dotId),
  });
  if (query.isError) return <ErrorNote error={query.error} />;
  const rows = (query.data ?? []).filter(
    (m) => props.root === undefined || m.root_run_id === props.root,
  );
  const currentRoot = props.root === undefined ? currentTopic(props.topics)?.rootId : undefined;
  return (
    <>
      {rows.map((message) => (
        <ScheduledCard
          key={message.id}
          message={message}
          dotName={props.dotName}
          target={scheduledTarget(message, props.topics, currentRoot)}
        />
      ))}
    </>
  );
}

/// A message waiting to be sent: when and where it goes, its text, a live countdown or why it
/// has not gone, and Send now, Edit and Cancel.
export function ScheduledCard(props: {
  message: ScheduledMessage;
  dotName: string;
  target: string;
}) {
  const { message } = props;
  const qc = useQueryClient();
  const now = useNow(message.status === "scheduled");
  const [editing, setEditing] = useState(false);
  const settle = () => {
    invalidate(qc, keys.scheduledMessages(message.dot_id));
    invalidate(qc, ["runs"]);
    invalidate(qc, ["thread"]);
  };
  const sendNow = useMutation({
    mutationFn: () => commands.sendScheduledNow(message.id),
    onSettled: settle,
  });
  const cancel = useMutation({
    mutationFn: () => commands.cancelScheduledMessage(message.id),
    onSettled: settle,
  });
  const edit = useMutation({
    mutationFn: ({ text, dueAt }: { text: string; dueAt: string }) =>
      commands.editScheduledMessage(message.id, text, dueAt),
    onSuccess: () => setEditing(false),
    onSettled: settle,
  });
  const all = [sendNow, cancel, edit];
  // Each press clears the others' errors, so the alert shows the latest failure only.
  const press = (own: (typeof all)[number], run: () => void) => {
    for (const m of all) if (m !== own) m.reset();
    run();
  };
  const busy = all.some((m) => m.isPending);
  const failed = all.find((m) => m.isError);

  const due = Date.parse(message.due_at);
  const when =
    due - now <= WEEK_MS ? localWeekdayTime(message.due_at, now) : localDateTime(message.due_at);
  const status =
    message.status === "failed" ? (
      <p className="text-xs text-needs">Not sent: {message.error}</p>
    ) : (
      <p className="text-xs text-muted-foreground">
        {message.status === "sending"
          ? "Sending…"
          : message.waiting !== null
            ? waitingText(message.waiting, props.dotName)
            : sendsIn(message.due_at, now)}
      </p>
    );
  return (
    <div role="region" aria-label="Scheduled message" className="flex flex-col items-end gap-1.5">
      <p className="flex items-center gap-1.5 self-end text-xs text-muted-foreground">
        <Clock aria-hidden className="size-3.5" />
        Scheduled for {when} · {props.target}
      </p>
      {editing ? (
        <EditForm
          message={message}
          pending={edit.isPending}
          onSave={(text, dueAt) => press(edit, () => edit.mutate({ text, dueAt }))}
          onDiscard={() => {
            edit.reset();
            setEditing(false);
          }}
        />
      ) : (
        <p className="max-w-[75%] self-end rounded-2xl rounded-br-sm border border-dashed bg-bubble/60 px-3.5 py-2 break-words whitespace-pre-wrap text-bubble-foreground">
          {message.text}
        </p>
      )}
      {status}
      {message.status !== "sending" && !editing && (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="xs"
            variant="outline"
            disabled={busy}
            onClick={() => press(sendNow, () => sendNow.mutate())}
          >
            Send now
          </Button>
          <Button
            size="xs"
            variant="ghost"
            disabled={busy}
            onClick={() => {
              for (const m of all) m.reset();
              setEditing(true);
            }}
          >
            Edit
          </Button>
          <Button
            size="xs"
            variant="ghost"
            disabled={busy}
            onClick={() => press(cancel, () => cancel.mutate())}
          >
            Cancel
          </Button>
        </div>
      )}
      {failed && (
        <p role="alert" className="text-xs text-destructive">
          {errorMessage(failed.error)}
        </p>
      )}
    </div>
  );
}

/// The time the Edit form starts with: the message's own, or an hour from `now` for one that is
/// due or within two minutes of it (its own time would be refused: the core takes a time at
/// least a minute ahead, and the field drops the seconds).
export function editTime(dueAt: string, now: number): Date {
  const due = Date.parse(dueAt);
  return due - now < 2 * 60_000 ? new Date(now + 3_600_000) : new Date(due);
}

/// Edit: the text and the time, saved together.
function EditForm(props: {
  message: ScheduledMessage;
  pending: boolean;
  onSave: (text: string, dueAt: string) => void;
  onDiscard: () => void;
}) {
  const textId = useId();
  const atId = useId();
  const [text, setText] = useState(props.message.text);
  const [at, setAt] = useState(() => toPicker(editTime(props.message.due_at, Date.now())));
  const picked = fromPicker(at);
  return (
    <form
      className="flex w-full max-w-[75%] flex-col gap-2 self-end"
      onSubmit={(e) => {
        e.preventDefault();
        if (picked !== null) props.onSave(text, picked.toISOString());
      }}
    >
      <Label htmlFor={textId}>Scheduled message</Label>
      <Textarea id={textId} value={text} onChange={(e) => setText(e.target.value)} />
      <Label htmlFor={atId}>Send at</Label>
      <Input
        id={atId}
        type="datetime-local"
        value={at}
        onChange={(e) => setAt(e.target.value)}
      />
      <div className="flex gap-2">
        <Button
          type="submit"
          size="xs"
          disabled={props.pending || picked === null || text.trim() === ""}
        >
          Save
        </Button>
        <Button type="button" size="xs" variant="ghost" onClick={props.onDiscard}>
          Discard changes
        </Button>
      </div>
    </form>
  );
}
