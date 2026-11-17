import { ChevronRight, Copy, Sparkles, Terminal } from "lucide-react";
import { toast } from "sonner";

import { notifyError } from "@/lib/notify";
import { rawSummary, rawView, tokenCount, type RawLine, type RawView } from "@/lib/raw-line";

type Thinking = Extract<RawView, { kind: "thinking" }>;

/// The thinking of a group: how much, and a small bar for each count the engine sent, so that
/// the pace of the thinking shows.
function ThinkingRow({ counts }: { counts: Thinking[] }) {
  const tokens = counts.reduce((sum, count) => sum + count.tokens, 0);
  const most = Math.max(1, ...counts.map((count) => count.tokens));
  return (
    <div className="flex items-end justify-between gap-4 p-2.5">
      <div className="flex min-w-0 flex-col gap-0.5">
        <p className="font-medium text-foreground">Thinking</p>
        <p>
          The model thought before it answered
          {tokens > 0 && <>: about {tokenCount(tokens)} tokens</>}.
        </p>
      </div>
      {counts.length > 1 && (
        <div aria-hidden className="flex h-7 shrink-0 items-end gap-0.5">
          {counts.map((count, i) => (
            <span
              key={i}
              title={`+${count.tokens} tokens`}
              className="w-1.5 rounded-sm bg-primary/60"
              style={{ height: `${Math.max(12, (count.tokens / most) * 100)}%` }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/// One line that is not thinking: its title (left out when the group's line already says it),
/// a button that copies the line as it came, and its fields or its text.
function LineRow(props: { line: RawLine; view: Exclude<RawView, Thinking>; titled: boolean }) {
  const { line, view, titled } = props;
  return (
    <div className="relative flex flex-col gap-1.5 p-2.5">
      {titled && <p className="pr-8 font-medium text-foreground">{view.title}</p>}
      <button
        type="button"
        aria-label="Copy the line"
        title="Copy the line"
        className="absolute top-1.5 right-1.5 rounded-md p-1 hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        onClick={() =>
          navigator.clipboard.writeText(line.text).then(() => toast.success("Copied"), notifyError)
        }
      >
        <Copy aria-hidden className="size-3.5" />
      </button>
      {view.kind === "text" ? (
        <pre className="max-h-60 overflow-auto pr-8 font-mono whitespace-pre-wrap">{view.text}</pre>
      ) : view.fields.length === 0 ? (
        <p className="pr-8">Nothing more in this line.</p>
      ) : (
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 pr-8">
          {view.fields.map((field) => (
            <div key={field.key} className="col-span-2 grid grid-cols-subgrid">
              <dt>{field.key}</dt>
              <dd className="max-h-40 overflow-auto font-mono break-words whitespace-pre-wrap text-foreground">
                {field.value}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}

/// A group of consecutive raw lines of a run (what the engine printed that is not conversation):
/// one muted line that says what they are, and opens to the thinking and to each other line as
/// its fields.
export function RawLines({ items }: { items: (RawLine & { seq: number })[] }) {
  const rows = items.map((line) => ({ line, view: rawView(line) }));
  const thinking = rows.map((row) => row.view).filter((view) => view.kind === "thinking");
  const rest = rows.filter((row) => row.view.kind !== "thinking");
  const Icon = rest.length === 0 ? Sparkles : Terminal;
  return (
    <details data-slot="raw" className="group/raw">
      <summary className="flex cursor-pointer list-none items-center gap-1 text-xs text-muted-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight
          aria-hidden
          className="size-3.5 shrink-0 transition-transform group-open/raw:rotate-90 motion-reduce:transition-none"
        />
        <Icon aria-hidden className="size-3.5 shrink-0" />
        <span className="min-w-0 truncate">{rawSummary(rows.map((row) => row.view))}</span>
      </summary>
      <div className="mt-1.5 flex flex-col divide-y rounded-lg border text-xs text-muted-foreground">
        {thinking.length > 0 && <ThinkingRow counts={thinking} />}
        {rest.map(({ line, view }) => (
          <LineRow
            key={line.seq}
            line={line}
            view={view as Exclude<RawView, Thinking>}
            titled={rows.length > 1}
          />
        ))}
      </div>
    </details>
  );
}
