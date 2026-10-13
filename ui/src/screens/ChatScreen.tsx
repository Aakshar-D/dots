import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import { Plus, Send } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { Run, RunEventRecord } from "@/api/types";
import { ApprovalCard } from "@/components/ApprovalCard";
import { ErrorNote, LoadErrors } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { StatusBadge } from "@/components/StatusBadge";
import { Transcript } from "@/components/Transcript";
import { Button } from "@/components/ui/button";
import { NativeSelect } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { chatThreads, isActive } from "@/lib/chat-threads";
import { relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { eventsQuery, invalidate, keys } from "@/lib/query";

/// Where a message is typed. Enter sends; Shift+Enter starts a new line. The text is cleared
/// once the send has succeeded; a refused message stays for another try.
function Composer(props: {
  disabled: boolean;
  hint: string;
  onSend: (message: string) => Promise<unknown>;
}) {
  const [text, setText] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);
  // The box is disabled while a send is on its way and while the dot works, and a browser
  // takes the focus from a disabled control. When the box can be used again it gets the focus
  // back, unless the user has put it somewhere else in the meantime.
  const wasDisabled = useRef(props.disabled);
  useEffect(() => {
    const freed = wasDisabled.current && !props.disabled;
    wasDisabled.current = props.disabled;
    const focused = document.activeElement;
    if (freed && (focused === null || focused === document.body || focused === box.current)) {
      box.current?.focus();
    }
  }, [props.disabled]);
  function send() {
    const message = text.trim();
    if (message === "" || props.disabled) return;
    // The mutation's onError reports a failure; here it only keeps the text.
    props.onSend(message).then(
      () => setText(""),
      () => {},
    );
  }
  return (
    <div className="flex items-end gap-2 border-t p-3">
      <label htmlFor="chat-message" className="sr-only">
        Message
      </label>
      <Textarea
        id="chat-message"
        ref={box}
        rows={2}
        placeholder={props.hint}
        disabled={props.disabled}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
      />
      <Button disabled={props.disabled || text.trim() === ""} onClick={send}>
        <Send aria-hidden /> Send
      </Button>
    </div>
  );
}

/// One conversation: its runs as a transcript, approvals it waits for, and the composer.
function Conversation({ dotId, rootId }: { dotId: string; rootId: string }) {
  const qc = useQueryClient();
  const thread = useQuery({
    queryKey: keys.thread(rootId),
    queryFn: () => commands.runThread(rootId),
  });
  const runs: Run[] = thread.data ?? [];
  const events = useQueries({ queries: runs.map((run) => eventsQuery(qc, run.id)) });
  const approvals = useQueries({
    queries: runs.map((run) => ({
      queryKey: keys.approvals(run.id),
      queryFn: () => commands.approvalsForRun(run.id),
    })),
  });
  const latest = runs.at(-1);
  // A follow-up continues from the thread's last run, so it has to have ended.
  const busy = runs.some(isActive);
  // And it resumes that run's session. A run that ended before its engine had one (a failed
  // start, a cancel while queued) leaves nothing to resume: the core refuses every follow-up.
  const closed = !busy && latest !== undefined && latest.session_id === null;
  // Loads the dot's chats and the thread again. The thread's promise is returned for the send
  // to wait on. That needs the promise `invalidate` does not give, and not its care for a
  // fetch in flight: the thread is on screen with its runs, so a refetch on its way is started
  // again.
  const reload = () => {
    invalidate(qc, ["runs"]);
    return qc.invalidateQueries({ queryKey: ["thread"] });
  };
  const send = useMutation({
    mutationFn: (message: string) => commands.chat(dotId, message, latest!.id),
    // Returned, so the send stays pending until the thread has reloaded: until then the
    // thread still ends at the old run, and a second message would continue from it.
    onSuccess: reload,
    // The core refuses a follow-up when the thread it holds is ahead of this one: a newer
    // message, a run that still works or waits for an approval, a last run without a session.
    // So the thread is loaded again after any refusal, and the composer follows what it brings;
    // until then a second try would only be refused the same way.
    onError: (error) => {
      notifyError(error);
      return reload();
    },
  });
  if (thread.isPending) return null;
  if (thread.isError) return <ErrorNote error={thread.error} />;
  const eventsByRun = new Map<string, RunEventRecord[]>(
    runs.map((run, i) => [run.id, events[i]?.data ?? []]),
  );
  const pending = approvals.flatMap((q) => q.data ?? []).filter((a) => a.status === "pending");
  // Why the last run failed stands below the conversation, so the transcript does not repeat it.
  const failure = latest?.status === "failed" ? latest.error : null;
  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto p-4">
        <Transcript runs={runs} events={eventsByRun} shownError={failure} />
        <LoadErrors queries={[...events, ...approvals]} />
        {pending.map((approval) => (
          <ApprovalCard key={approval.id} approval={approval} />
        ))}
        {failure && (
          <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
            {failure}
          </p>
        )}
        {closed && (
          <p className="text-muted-foreground">
            This chat cannot be continued: its last run ended before the dot had a session to
            resume.{" "}
            <Link to={`/chat/${dotId}`} className="underline">
              Start a new chat
            </Link>
            .
          </p>
        )}
      </div>
      <Composer
        disabled={busy || send.isPending || !latest || closed}
        hint={
          busy
            ? "The dot is working…"
            : closed
              ? "This chat cannot be continued"
              : "Reply to continue this chat"
        }
        onSend={(message) => send.mutateAsync(message)}
      />
    </>
  );
}

/// A new conversation: the first message starts a chat run, which becomes the thread.
function NewConversation({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const start = useMutation({
    mutationFn: (message: string) => commands.chat(dotId, message, null),
    onSuccess: (run) => {
      invalidate(qc, ["runs"]);
      void navigate(`/chat/${dotId}/${run.id}`);
    },
    onError: notifyError,
  });
  return (
    <>
      <p className="flex-1 p-4 text-muted-foreground">
        Ask the dot something. It answers with its own instructions, folder and permissions.
      </p>
      <Composer
        disabled={start.isPending}
        hint="Message the dot"
        onSend={(message) => start.mutateAsync(message)}
      />
    </>
  );
}

function Threads({ dotId, rootId }: { dotId: string; rootId: string | undefined }) {
  const runs = useQuery({
    // Its own key under "runs": the dot editor lists the same dot's runs with another limit.
    queryKey: [...keys.runs(dotId), "chat"],
    queryFn: () => commands.listRuns(dotId, 200),
  });
  const threads = chatThreads(runs.data ?? []);
  return (
    <nav aria-label="Chats" className="flex flex-col gap-1">
      <Button asChild size="sm" variant="outline">
        <Link to={`/chat/${dotId}`}>
          <Plus aria-hidden /> New chat
        </Link>
      </Button>
      {runs.isError && <ErrorNote error={runs.error} />}
      {threads.map((thread) => (
        <Link
          key={thread.rootId}
          to={`/chat/${dotId}/${thread.rootId}`}
          aria-current={thread.rootId === rootId ? "page" : undefined}
          className={cn(
            "flex flex-col gap-0.5 rounded-md px-2 py-1.5 hover:bg-accent",
            thread.rootId === rootId && "bg-accent",
          )}
        >
          <span className="truncate">{thread.title}</span>
          <span className="flex items-center gap-2 text-xs text-muted-foreground">
            {isActive(thread.latest) && <StatusBadge status={thread.latest.status} />}
            {relativeTime(thread.latest.queued_at)}
          </span>
        </Link>
      ))}
    </nav>
  );
}

/// Chat with a dot: pick the dot, then one of its conversations or a new one.
export function ChatScreen() {
  const { dotId, runId } = useParams();
  const navigate = useNavigate();
  const dots = useQuery({ queryKey: keys.dots, queryFn: commands.listDots });
  if (dots.isPending) return <Page title="Chat" />;
  if (dots.isError) {
    return (
      <Page title="Chat">
        <ErrorNote error={dots.error} />
      </Page>
    );
  }
  if (dots.data.length === 0) {
    return (
      <Page title="Chat">
        <p className="text-muted-foreground">
          There is no dot to chat with yet. <Link to="/dots/new" className="underline">Create one</Link>.
        </p>
      </Page>
    );
  }
  return (
    <div className="flex h-full">
      <aside className="flex w-60 shrink-0 flex-col gap-3 border-r p-3">
        <h1 className="text-lg font-semibold">Chat</h1>
        <label htmlFor="chat-dot" className="sr-only">
          Dot
        </label>
        <NativeSelect
          id="chat-dot"
          className="w-full"
          value={dotId ?? ""}
          onChange={(e) => void navigate(e.target.value ? `/chat/${e.target.value}` : "/chat")}
        >
          <option value="">Choose a dot…</option>
          {dots.data.map((dot) => (
            <option key={dot.id} value={dot.id}>
              {dot.name}
            </option>
          ))}
        </NativeSelect>
        {dotId && <Threads dotId={dotId} rootId={runId} />}
      </aside>
      <section aria-label="Conversation" className="flex min-w-0 flex-1 flex-col">
        {!dotId ? (
          <p className="p-4 text-muted-foreground">Choose a dot to chat with.</p>
        ) : runId ? (
          <Conversation key={runId} dotId={dotId} rootId={runId} />
        ) : (
          <NewConversation key={dotId} dotId={dotId} />
        )}
      </section>
    </div>
  );
}
