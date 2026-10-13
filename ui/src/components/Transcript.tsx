import type { Run, RunEventRecord } from "@/api/types";
import { Markdown } from "@/components/Markdown";
import { StatusBadge } from "@/components/StatusBadge";
import { localDateTime } from "@/lib/format";
import { toolPreview, transcriptItems, type TranscriptItem } from "@/lib/transcript";

const TRIGGER: Record<Run["trigger"], string> = {
  schedule: "Scheduled run",
  webhook: "Webhook run",
  manual: "Manual run",
  chat: "Chat message",
  resume: "Continued after approval",
};

/// What the person wrote, for a chat run.
function chatMessage(run: Run): string | null {
  if (run.trigger !== "chat" || typeof run.payload !== "object" || run.payload === null) {
    return null;
  }
  const message = (run.payload as Record<string, unknown>).message;
  return typeof message === "string" ? message : null;
}

function Item({ item }: { item: TranscriptItem }) {
  switch (item.type) {
    case "text":
      return <Markdown text={item.text} />;
    case "error":
      return (
        <p role="alert" className="rounded-md border border-destructive/50 p-2 text-destructive">
          {item.text}
        </p>
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
            <span className="font-semibold">{item.tool}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-muted-foreground">
              {toolPreview(item.input)}
            </span>
            <span className={item.result?.isError ? "text-destructive" : "text-muted-foreground"}>
              {item.result === null ? "running" : item.result.isError ? "failed" : "done"}
            </span>
          </summary>
          <pre className="mt-1 overflow-x-auto text-xs whitespace-pre-wrap">
            {JSON.stringify(item.input, null, 2)}
          </pre>
          {item.result && (
            <pre
              className={`mt-1 max-h-80 overflow-auto border-t pt-1 text-xs whitespace-pre-wrap ${
                item.result.isError ? "text-destructive" : "text-muted-foreground"
              }`}
            >
              {item.result.output}
            </pre>
          )}
        </details>
      );
  }
}

/// A thread as one conversation: each run of the lineage in order, with its events.
export function Transcript(props: { runs: Run[]; events: Map<string, RunEventRecord[]> }) {
  const many = props.runs.length > 1;
  return (
    <div className="flex flex-col gap-4">
      {props.runs.map((run) => {
        const message = chatMessage(run);
        const items = transcriptItems(props.events.get(run.id) ?? []);
        return (
          <section key={run.id} aria-label={`${TRIGGER[run.trigger]}, ${localDateTime(run.queued_at)}`}>
            <header className="mb-2 flex items-center gap-2 text-xs text-muted-foreground">
              <span>{TRIGGER[run.trigger]}</span>
              <span>{localDateTime(run.queued_at)}</span>
              {many && <StatusBadge status={run.status} />}
            </header>
            <div className="flex flex-col gap-2">
              {message !== null && (
                <p className="self-end rounded-lg bg-primary px-3 py-1.5 whitespace-pre-wrap text-primary-foreground">
                  {message}
                </p>
              )}
              {items.map((item) => (
                <Item key={item.seq} item={item} />
              ))}
              {items.length === 0 && message === null && run.status === "queued" && (
                <p className="text-muted-foreground">Waiting to start.</p>
              )}
            </div>
          </section>
        );
      })}
    </div>
  );
}
