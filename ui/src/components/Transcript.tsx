import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Info } from "lucide-react";
import { Link } from "react-router";
import { z } from "zod";

import { commands } from "@/api/commands";
import {
  LIMITS,
  type LibraryEntry,
  type Message,
  type Run,
  type RunEventRecord,
} from "@/api/types";
import { BrowserCall } from "@/components/BrowserCall";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
import { isActive } from "@/lib/home-topics";
import { localDateTime } from "@/lib/format";
import { mcpToolLabel } from "@/lib/mcp-tool";
import { keys } from "@/lib/query";
import {
  checkInLine,
  toolPreview,
  transcriptItems,
  type TranscriptItem,
} from "@/lib/transcript";

const TRIGGER: Record<Run["trigger"], string> = {
  schedule: "Scheduled run",
  webhook: "Webhook run",
  manual: "Manual run",
  chat: "Chat message",
  resume: "Continued after approval",
  follow_up: "Follow-up the dot scheduled",
};

/// What the person wrote, for a chat run: an answer to a question as they typed it, not the
/// words Dots wraps it in for the dot.
function chatMessage(run: Run): string | null {
  if (run.trigger !== "chat" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const { answer, message } = run.payload as Record<string, unknown>;
  if (typeof answer === "string") return answer;
  return typeof message === "string" ? message : null;
}

/// What the scheduler wrote into the payload of a run it started: the slot, whether the slot
/// had been missed, and which attempt of the occurrence the run is (a first one has none).
const SchedulePayload = z.object({
  scheduled_for: z.string(),
  caught_up: z.boolean().optional(),
  attempt: z.number().int().optional(),
});

/// The line under a scheduled run's trigger: the slot it served, or the attempt it retries,
/// with a link to the run that failed. Nothing for another trigger or a payload it cannot read.
function ScheduleLine({ run }: { run: Run }) {
  if (run.trigger !== "schedule") return null;
  const payload = SchedulePayload.safeParse(run.payload);
  if (!payload.success) return null;
  const { scheduled_for: slot, caught_up: caughtUp, attempt } = payload.data;
  let line: ReactNode = `Scheduled for ${localDateTime(slot)}`;
  if (caughtUp) line = `${line}, caught up after the PC was off`;
  const of = LIMITS.max_schedule_attempts;
  if (attempt !== undefined && attempt > 1) {
    // A retry names the run it retries. A slot that dropped a pending retry carries its attempt
    // without `retry_of`: it still serves its own slot.
    line =
      run.retry_of === null ? (
        `${line} · attempt ${attempt} of ${of}`
      ) : (
        <>
          Retry {attempt} of {of} of{" "}
          <Link to={`/runs/${run.retry_of}`} className="text-foreground hover:underline">
            the failed run
          </Link>
        </>
      );
  }
  return <p className="mb-2 text-xs text-muted-foreground">{line}</p>;
}

/// The message a follow-up run carries: what the dot scheduled itself to do.
function FollowUpLine({ run }: { run: Run }) {
  if (run.trigger !== "follow_up" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const message = (run.payload as Record<string, unknown>).message;
  if (typeof message !== "string") return null;
  return <p className="mb-2 text-xs whitespace-pre-wrap text-muted-foreground">{message}</p>;
}

/// The result Dots gives a call it parked for a person's approval (`approvals::parked_message`).
const PARKED = "Queued for human approval #";

/// Whether a call's result says it was parked for approval: not a failure.
function parked(result: { output: string; isError: boolean } | null): boolean {
  return result !== null && result.output.startsWith(PARKED);
}

/// What became of a call: running, done, sent for approval, failed, or no result in a run that
/// has ended.
function outcome(result: { output: string; isError: boolean } | null, active: boolean): string {
  if (result === null) return active ? "running" : "no result";
  if (parked(result)) return "sent for approval";
  return result.isError ? "failed" : "done";
}

/// The colour of a call's outcome: a failure in the destructive colour, anything else muted.
function outcomeColour(result: { output: string; isError: boolean } | null): string {
  return result?.isError && !parked(result) ? "text-destructive" : "text-muted-foreground";
}

/// `active`: the item's run is still going. Only then can a tool call without a result be one
/// that runs; in a run that has ended, the result never came.
function Item(props: {
  item: TranscriptItem;
  active: boolean;
  /// MCP server name to the plugin it comes with.
  owners: ReadonlyMap<string, string>;
  /// The item's run, whose browser screenshots a browser call shows.
  runId: string;
}) {
  const { item, active, owners, runId } = props;
  switch (item.type) {
    case "text":
      return <Markdown text={item.text} />;
    case "error":
      return (
        <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
          {item.text}
        </p>
      );
    case "note":
      return (
        <p role="note" className="flex items-start gap-1.5 text-xs text-muted-foreground">
          <Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          {item.text}
        </p>
      );
    case "checkin":
      return (
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="min-w-0 flex-1 truncate">
              {checkInLine(item.kind, item.input, item.result)}
            </span>
            <span className={outcomeColour(item.result)}>{outcome(item.result, active)}</span>
          </summary>
          {item.result && (
            <pre
              className={`mt-1 max-h-80 overflow-auto text-xs whitespace-pre-wrap ${
                item.result.isError ? "text-destructive" : "text-muted-foreground"
              }`}
            >
              {item.result.output}
            </pre>
          )}
        </details>
      );
    case "browser":
      return (
        <BrowserCall
          runId={runId}
          tool={item.tool}
          input={item.input}
          result={item.result}
          active={active}
        />
      );
    case "raw":
      return (
        <details className="rounded-md border px-2 py-1 text-muted-foreground">
          <summary className="cursor-pointer text-xs">{item.label}</summary>
          <pre className="mt-1 overflow-x-auto text-xs whitespace-pre-wrap">{item.text}</pre>
        </details>
      );
    case "tool":
      return (
        <details className="rounded-md border px-2 py-1">
          <summary className="flex cursor-pointer items-center gap-2 text-xs">
            <span className="font-semibold" title={item.tool}>
              {mcpToolLabel(item.tool, owners)}
            </span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
              {toolPreview(item.input)}
            </span>
            <span className={outcomeColour(item.result)}>{outcome(item.result, active)}</span>
          </summary>
          <pre className="mt-1 overflow-x-auto text-xs whitespace-pre-wrap">
            {JSON.stringify(item.input, null, 2)}
          </pre>
          {item.result && (
            <pre
              className={`mt-1 max-h-80 overflow-auto border-t pt-1 text-xs whitespace-pre-wrap ${outcomeColour(
                item.result,
              )}`}
            >
              {item.result.output}
            </pre>
          )}
        </details>
      );
  }
}

/// Server name to plugin name, for the servers that come with a plugin.
function pluginOwners(entries: LibraryEntry[]): Map<string, string> {
  const names = new Map(entries.filter((e) => e.kind === "plugin").map((e) => [e.id, e.name]));
  const owners = new Map<string, string>();
  for (const entry of entries) {
    const plugin = entry.owner_id === null ? undefined : names.get(entry.owner_id);
    if (entry.kind === "mcp_server" && plugin) owners.set(entry.name, plugin);
  }
  return owners;
}

/// A note or a question the dot posted during a run, as a message from the dot (check-ins spec
/// section 9).
function CheckIn({ message }: { message: Message }) {
  const question = message.kind === "question";
  const state = !question
    ? null
    : message.answered_at
      ? "answered"
      : message.dismissed_at
        ? "dismissed"
        : "waiting for your answer";
  return (
    <div
      role="group"
      aria-label={question ? "Question from the dot" : "Note from the dot"}
      className="flex max-w-prose flex-col gap-1 self-start rounded-lg bg-muted px-3 py-1.5"
    >
      <p className="whitespace-pre-wrap">{message.text}</p>
      {state && <span className="text-xs text-muted-foreground">Question · {state}</span>}
    </div>
  );
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: {
  runs: Run[];
  events: Map<string, RunEventRecord[]>;
  /// The dot's notes and questions of these runs, oldest first; each shows after its run.
  messages?: Message[];
  /// The last run's error, when the screen shows it by itself. A failed run stores its error as
  /// an event too: the one of the last run that says the same is left out, so that the error is
  /// on screen once.
  shownError?: string | null;
}) {
  const many = props.runs.length > 1;
  // A plugin's MCP server shows as its plugin; the Library tells which server comes from which.
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const owners = pluginOwners(library.data ?? []);
  // Each run's section is a region named by its trigger and time; two runs in the same minute
  // get "(2)", "(3)" so that their names differ.
  const seen = new Map<string, number>();
  const names = props.runs.map((run) => {
    const name = `${TRIGGER[run.trigger]}, ${localDateTime(run.queued_at)}`;
    const n = (seen.get(name) ?? 0) + 1;
    seen.set(name, n);
    return n === 1 ? name : `${name} (${n})`;
  });
  return (
    <div className="flex flex-col gap-4">
      {props.runs.map((run, i) => {
        const message = chatMessage(run);
        const last = i === props.runs.length - 1;
        const items = transcriptItems(props.events.get(run.id) ?? []).filter(
          (item) => !(last && item.type === "error" && item.text === props.shownError),
        );
        return (
          <section key={run.id} aria-label={names[i]}>
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{TRIGGER[run.trigger]}</span>
              <span>{localDateTime(run.queued_at)}</span>
              {many && <StatusBadge status={run.status} />}
            </header>
            <ScheduleLine run={run} />
            <FollowUpLine run={run} />
            <div className="flex flex-col gap-2">
              {message !== null && (
                <p className="self-end rounded-lg bg-primary px-3 py-1.5 whitespace-pre-wrap text-primary-foreground">
                  {message}
                </p>
              )}
              {items.map((item) => (
                <Item
                  key={item.seq}
                  item={item}
                  active={isActive(run)}
                  owners={owners}
                  runId={run.id}
                />
              ))}
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
              {(props.messages ?? [])
                .filter((checkIn) => checkIn.run_id === run.id)
                .map((checkIn) => (
                  <CheckIn key={checkIn.id} message={checkIn} />
                ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
