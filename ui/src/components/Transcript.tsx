import { Fragment, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { cn } from "cn";
import { Brain, Info } from "lucide-react";
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
import { appTitle, isActive } from "@/lib/home-topics";
import { localDateTime, localTime } from "@/lib/format";
import { mcpToolLabel } from "@/lib/mcp-tool";
import { keys } from "@/lib/query";
import { watcherTitle } from "@/lib/watchers";
import {
  checkInLine,
  callLabel,
  memoryNote,
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
  app: "Started by an app",
  workflow: "Workflow step",
  watcher: "Watcher run",
};

/// What a run's header says started it; an app-started run names its app (app library spec
/// section 7).
export function runTrigger(run: Pick<Run, "trigger" | "payload">): string {
  const title = appTitle(run);
  return title === null ? TRIGGER[run.trigger] : `Started by ${title}`;
}

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

/// What woke the dot, under a watcher run's header (watchers spec section 8).
function WatcherLine({ run }: { run: Run }) {
  const title = watcherTitle(run);
  if (title === null) return null;
  return <p className="mb-2 text-xs text-muted-foreground">{title}</p>;
}

/// The message a follow-up run carries: what the dot scheduled itself to do; and the message an
/// app started a run with.
function FollowUpLine({ run }: { run: Run }) {
  const carries = run.trigger === "follow_up" || run.trigger === "app";
  if (!carries || typeof run.payload !== "object" || run.payload === null) {
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

/// A failure in the conversation (deep-teal spec section 4): soft red text with a rule on its
/// left, not a filled box, and still read out at once.
export function Failure({ children }: { children: ReactNode }) {
  return (
    <p
      role="alert"
      data-slot="failure"
      className="border-l-2 border-destructive py-0.5 pl-3 whitespace-pre-wrap text-destructive"
    >
      {children}
    </p>
  );
}

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

/// The mark at the end of a tool step's chip; the outcome's word is there for assistive
/// technology.
const GLYPH: Record<string, string> = {
  done: "✓",
  failed: "✕",
  running: "…",
  "no result": "–",
  "sent for approval": "!",
};

/// One tool step as a small rounded chip (its tool, then a mark) that opens to the call and its
/// result. A call sent for approval has its chip in the needs colour.
function ToolChip(props: { item: ToolItem; active: boolean; owners: ReadonlyMap<string, string> }) {
  const { item, active, owners } = props;
  const said = outcome(item.result, active);
  const tone = parked(item.result) ? "needs" : item.result?.isError ? "failed" : "plain";
  const mark =
    tone === "failed"
      ? "text-destructive"
      : tone === "needs"
        ? undefined
        : said === "done"
          ? "text-success"
          : "text-muted-foreground";
  return (
    <details data-tone={tone} className="open:basis-full">
      <summary
        className={cn(
          "inline-flex cursor-pointer list-none items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs [&::-webkit-details-marker]:hidden",
          tone === "needs" ? "border-needs-border bg-needs-surface text-needs" : "bg-muted",
        )}
      >
        <span className="font-medium" title={item.tool}>
          {mcpToolLabel(callLabel(item.tool, item.input), owners)}
        </span>
        <span aria-hidden className={mark}>
          {GLYPH[said]}
        </span>
        <span className="sr-only">{said}</span>
      </summary>
      <div className="mt-1.5 flex flex-col gap-1 rounded-lg border p-2">
        <p className="truncate font-mono text-xs text-muted-foreground">{toolPreview(item.input)}</p>
        <pre className="overflow-x-auto text-xs whitespace-pre-wrap">
          {JSON.stringify(item.input, null, 2)}
        </pre>
        {item.result && (
          <pre
            className={`max-h-80 overflow-auto border-t pt-1 text-xs whitespace-pre-wrap ${outcomeColour(
              item.result,
            )}`}
          >
            {item.result.output}
          </pre>
        )}
      </div>
    </details>
  );
}

/// The line under the chip of a `remember` or `forget` that was done (dot memory spec section
/// 5): what the dot remembered, or that it forgot one, and a link to the dot's Memory tab.
function MemoryNote({ item, dotId }: { item: ToolItem; dotId: string }) {
  const note = memoryNote(item.tool, item.input, item.result);
  if (note === null) return null;
  return (
    <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <Brain aria-hidden className="size-3.5 shrink-0" />
      <span className="min-w-0 truncate">
        {note.kind === "remembered" ? `Remembered: ${note.text}` : "Forgot a memory"}
      </span>
      <Link to={`/dots/${dotId}?tab=memory`} className="text-foreground hover:underline">
        Memory
      </Link>
    </p>
  );
}

/// What a run's section shows, in order: consecutive tool steps as one row of chips, every
/// other item by itself.
type Block =
  | { kind: "tools"; seq: number; items: ToolItem[] }
  | { kind: "item"; item: TranscriptItem };

function blocks(items: TranscriptItem[]): Block[] {
  const out: Block[] = [];
  for (const item of items) {
    const last = out.at(-1);
    if (item.type === "tool" && last?.kind === "tools") last.items.push(item);
    else if (item.type === "tool") out.push({ kind: "tools", seq: item.seq, items: [item] });
    else out.push({ kind: "item", item });
  }
  return out;
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
      return <Failure>{item.text}</Failure>;
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
      return <ToolChip item={item} active={active} owners={owners} />;
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
/// section 9): a soft card, with its kind and a question's state in its meta line.
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
      className="flex max-w-prose flex-col gap-1 self-start rounded-xl bg-muted px-3.5 py-2.5"
    >
      <span className="text-xs text-muted-foreground">
        {state ? `Question · ${state}` : "Note"}
      </span>
      <p className="whitespace-pre-wrap">{message.text}</p>
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
  /// The dot's name, for the meta line above its words.
  dotName?: string;
}) {
  const many = props.runs.length > 1;
  // A plugin's MCP server shows as its plugin; the Library tells which server comes from which.
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const owners = pluginOwners(library.data ?? []);
  // Each run's section is a region named by its trigger and time; two runs in the same minute
  // get "(2)", "(3)" so that their names differ.
  const seen = new Map<string, number>();
  const names = props.runs.map((run) => {
    const name = `${runTrigger(run)}, ${localDateTime(run.queued_at)}`;
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
        const checkIns = (props.messages ?? []).filter((checkIn) => checkIn.run_id === run.id);
        return (
          <section key={run.id} aria-label={names[i]}>
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{runTrigger(run)}</span>
              <span>{localDateTime(run.queued_at)}</span>
              {many && <StatusBadge status={run.status} />}
            </header>
            <ScheduleLine run={run} />
            <FollowUpLine run={run} />
            <WatcherLine run={run} />
            <div className="flex flex-col gap-2">
              {message !== null && (
                <p
                  data-from="person"
                  className="max-w-[80%] self-end rounded-2xl rounded-br-sm bg-bubble px-3.5 py-2 whitespace-pre-wrap text-bubble-foreground"
                >
                  {message}
                </p>
              )}
              {(items.length > 0 || checkIns.length > 0) && (
                <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                  <span aria-hidden className="size-1.5 rounded-full bg-primary" />
                  {props.dotName ?? "The dot"} · {localTime(run.queued_at)}
                </p>
              )}
              {blocks(items).map((block) =>
                block.kind === "tools" ? (
                  <Fragment key={block.seq}>
                    <div
                      role="group"
                      aria-label="Tool steps"
                      className="flex flex-wrap items-start gap-1.5"
                    >
                      {block.items.map((item) => (
                        <ToolChip
                          key={item.seq}
                          item={item}
                          active={isActive(run)}
                          owners={owners}
                        />
                      ))}
                    </div>
                    {block.items.map((item) => (
                      <MemoryNote key={item.seq} item={item} dotId={run.dot_id} />
                    ))}
                  </Fragment>
                ) : (
                  <Item
                    key={block.item.seq}
                    item={block.item}
                    active={isActive(run)}
                    owners={owners}
                    runId={run.id}
                  />
                ),
              )}
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
              {checkIns.map((checkIn) => (
                <CheckIn key={checkIn.id} message={checkIn} />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}
