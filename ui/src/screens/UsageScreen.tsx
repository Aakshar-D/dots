import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Fragment, useEffect, useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ClaudeTotals, LimitReading, UsagePeriod, UsageRow } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { SpendLine, useSpendToday } from "@/components/SpendCaps";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { localDateTime, localTime, relativeTime } from "@/lib/format";
import { triggerLabel } from "@/lib/home-topics";
import { keys } from "@/lib/query";
import {
  count,
  formatCost,
  formatDuration,
  formatTokens,
  pct,
  projectNames,
} from "@/lib/usage-format";

const PERIODS: { value: UsagePeriod; tab: string; span: string }[] = [
  { value: "five_hours", tab: "5 hours", span: "5 hours" },
  { value: "week", tab: "Week", span: "7 days" },
  { value: "month", tab: "Month", span: "30 days" },
];

const cost = (usd: number | null) => (usd === null ? "–" : formatCost(usd));

/// One window: its share used and the share at which it holds background runs.
function Window(props: { label: string; reading: LimitReading | null; share: number }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="text-muted-foreground">{props.label}</span>
      <span className="font-medium">
        {props.reading ? `${pct(props.reading.utilization)}%` : "unknown"}
      </span>
      <span className="text-xs text-muted-foreground">holds at {props.share}%</span>
    </div>
  );
}

/// The current 5-hour block from Claude Code's transcripts (usage tab spec section 6): every
/// message, Dots' and the person's, since the window the CLI last reported started.
function BlockLine() {
  const block = useQuery({
    queryKey: keys.claudeUsage("five_hours"),
    queryFn: () => commands.claudeUsage("five_hours"),
  }).data?.block;
  if (!block) return null;
  return (
    <span className="text-sm text-muted-foreground">
      This block: {count(block.messages, "message")} · {formatTokens(block.tokens)} tokens since{" "}
      {localTime(block.started_at)}
    </span>
  );
}

/// The account's windows (the whole account, not only Dots' runs) and the held runs.
function Limits() {
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  if (!limits) return null;
  const held = limits.held_run_ids.length;
  return (
    <section aria-label="Claude usage" className="flex flex-wrap items-center gap-x-6 gap-y-2">
      <Window label="5-hour" reading={limits.five_hour} share={limits.shares.five_hour} />
      <BlockLine />
      <Window label="weekly" reading={limits.weekly} share={limits.shares.weekly} />
      {held > 0 && (
        <Link to="/inbox" className="text-sm underline underline-offset-4">
          Holding {count(held, "run")}
        </Link>
      )}
    </section>
  );
}

/// Today's and this month's spend of every dot against the spending caps (spend caps design),
/// and each dot that has a cap of its own.
function Spending() {
  const spend = useSpendToday().data;
  if (!spend) return null;
  const capped = spend.dots.filter(
    (d) => d.caps.daily_usd !== null || d.caps.daily_tokens !== null,
  );
  return (
    <section aria-label="Spending" className="flex flex-col gap-1">
      <SpendLine
        label="Today, all dots"
        spend={spend.today}
        usdCap={spend.caps.daily_usd}
        tokenCap={spend.caps.daily_tokens}
        showTokens
      />
      <SpendLine label="This month" spend={spend.month} usdCap={spend.caps.monthly_usd} />
      {capped.map((d) => (
        <SpendLine
          key={d.dot_id}
          label={`Today, ${d.dot_name}`}
          spend={d.today}
          usdCap={d.caps.daily_usd}
          tokenCap={d.caps.daily_tokens}
        />
      ))}
      <Link to="/settings/runs" className="text-xs text-muted-foreground underline underline-offset-4">
        Spending caps
      </Link>
    </section>
  );
}

function DotRows(props: { row: UsageRow }) {
  const { row } = props;
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <Fragment>
      <tr className="border-t">
        <td className="py-1.5 pr-4">
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
            className="inline-flex items-center gap-1 rounded-sm text-left hover:underline"
          >
            <Chevron className="size-3.5 shrink-0" aria-hidden />
            {row.dot_name}
          </button>
        </td>
        <td className="py-1.5 pr-4 tabular-nums">{row.runs}</td>
        <td className="py-1.5 pr-4 tabular-nums">{formatTokens(row.tokens_in + row.tokens_out)}</td>
        <td className="py-1.5 pr-4 tabular-nums">{formatDuration(row.run_secs)}</td>
        <td className="py-1.5 tabular-nums">{cost(row.cost_usd)}</td>
      </tr>
      {open && (
        <tr>
          <td colSpan={5} className="pb-2 pl-6">
            <ul aria-label={`Heaviest runs of ${row.dot_name}`} className="flex flex-col gap-1">
              {row.heaviest.map((run) => (
                <li key={run.run_id}>
                  <Link to={`/runs/${run.run_id}`} className="text-sm hover:underline">
                    {[
                      triggerLabel(run),
                      localDateTime(run.started_at),
                      `${formatTokens(run.tokens_in + run.tokens_out)} tokens`,
                      run.run_secs === null ? null : formatDuration(run.run_secs),
                      run.cost_usd === null ? null : formatCost(run.cost_usd),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </Link>
                </li>
              ))}
            </ul>
          </td>
        </tr>
      )}
    </Fragment>
  );
}

function UsageTable(props: { period: UsagePeriod; span: string }) {
  const summary = useQuery({
    queryKey: keys.usage(props.period),
    queryFn: () => commands.usageSummary(props.period),
  });
  if (summary.isError) return <ErrorNote error={summary.error} />;
  if (!summary.data) return null;
  if (summary.data.rows.length === 0) {
    return <p className="text-muted-foreground">No runs in the last {props.span}.</p>;
  }
  const caption = `Dots' runs in the last ${props.span}`;
  return (
    // A box that may scroll sideways is a named stop for the keyboard (axe scrollable-region).
    <div
      className="max-w-full overflow-x-auto"
      tabIndex={0}
      role="region"
      aria-label={caption}
    >
      <table className="w-full text-left text-sm">
        <caption className="sr-only">{caption}</caption>
        <thead className="text-xs text-muted-foreground">
          <tr>
            <th scope="col" className="pb-1 pr-4 font-medium">Dot</th>
            <th scope="col" className="pb-1 pr-4 font-medium">Runs</th>
            <th scope="col" className="pb-1 pr-4 font-medium">Tokens</th>
            <th scope="col" className="pb-1 pr-4 font-medium">Run time</th>
            <th
              scope="col"
              className="pb-1 font-medium"
              title="Your subscription does not bill per run: this is what the run would cost on the API."
            >
              API-eq.
            </th>
          </tr>
        </thead>
        <tbody>
          {summary.data.rows.map((row) => (
            <DotRows key={row.dot_id} row={row} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

/// Input, cache creation and output: what "tokens" means in All Claude use. Cache reads are
/// shown on their own (usage tab spec section 6).
const tokens = (t: ClaudeTotals) => t.input_tokens + t.cache_creation_tokens + t.output_tokens;

const sum = (a: ClaudeTotals, b: ClaudeTotals): ClaudeTotals => ({
  messages: a.messages + b.messages,
  sessions: a.sessions + b.sessions,
  input_tokens: a.input_tokens + b.input_tokens,
  cache_creation_tokens: a.cache_creation_tokens + b.cache_creation_tokens,
  cache_read_tokens: a.cache_read_tokens + b.cache_read_tokens,
  output_tokens: a.output_tokens + b.output_tokens,
});

function Total(props: { label: string; totals: ClaudeTotals }) {
  const t = props.totals;
  return (
    <div className="flex flex-col">
      <dt className="text-xs text-muted-foreground">{props.label}</dt>
      <dd className="tabular-nums">
        {`${count(t.messages, "message")} · ${count(t.sessions, "session")} · ` +
          `${formatTokens(tokens(t))} tokens · ` +
          `cache reads ${formatTokens(t.cache_read_tokens)}`}
      </dd>
    </div>
  );
}

/// How often an open Usage page asks for a scan again, so it shows what a background scan (or
/// the person's own sessions) added meanwhile. The core answers a scan within 30 seconds of the
/// last one with that one's result, and otherwise reads only the bytes appended since.
export const CLAUDE_REFRESH_MS = 60_000;

/// Reads what Claude Code wrote since the last scan when the page opens, again at once while a
/// scan stops at its cap and every `CLAUDE_REFRESH_MS` while the page is open, and refreshes
/// All Claude use after each. True while scanning.
function useClaudeScan(): boolean {
  const client = useQueryClient();
  const scan = useQuery({
    queryKey: keys.claudeUsageScan,
    queryFn: commands.claudeUsageScan,
    staleTime: 0,
    refetchOnWindowFocus: false,
    refetchInterval: (q) => (q.state.data?.more ? 250 : CLAUDE_REFRESH_MS),
  });
  const at = scan.dataUpdatedAt;
  useEffect(() => {
    if (at) void client.invalidateQueries({ queryKey: ["claude-usage"] });
  }, [at, client]);
  return scan.isFetching || scan.data?.more === true;
}

/// The section's body: the totals and the project table, or why there are none.
function AllClaudeBody(props: { period: UsagePeriod; span: string }) {
  const usage = useQuery({
    queryKey: keys.claudeUsage(props.period),
    queryFn: () => commands.claudeUsage(props.period),
  });
  if (usage.isError) return <ErrorNote error={usage.error} />;
  const data = usage.data;
  if (!data) return null;
  if (!data.available) {
    return (
      <p className="text-muted-foreground">
        No Claude Code transcripts on this PC
        {data.folder ? ` (looked in ${data.folder})` : ""}.
      </p>
    );
  }
  const names = projectNames(data.projects);
  return (
    <>
      <dl className="flex flex-col gap-1">
        <Total label="Your own sessions" totals={data.own} />
        <Total label="Dots" totals={data.dots} />
        <Total label="All" totals={sum(data.own, data.dots)} />
      </dl>
      {data.projects.length === 0 ? (
        <p className="text-muted-foreground">No Claude Code use in the last {props.span}.</p>
      ) : (
        <div
          className="max-w-full overflow-x-auto"
          tabIndex={0}
          role="region"
          aria-label={`Claude use by project in the last ${props.span}`}
        >
          <table className="w-full text-left text-sm">
            <caption className="sr-only">Claude use by project in the last {props.span}</caption>
            <thead className="text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="pb-1 pr-4 font-medium">Project</th>
                <th scope="col" className="pb-1 pr-4 font-medium">Messages</th>
                <th scope="col" className="pb-1 pr-4 font-medium">Sessions</th>
                <th scope="col" className="pb-1 pr-4 font-medium">Tokens</th>
                <th scope="col" className="pb-1 font-medium">Cache reads</th>
              </tr>
            </thead>
            <tbody>
              {data.projects.map((p, i) => (
                <tr key={p.cwd ?? ""} className="border-t">
                  <td className="py-1.5 pr-4">
                    <span title={p.cwd ?? "Every dot's runs"}>{names[i]}</span>
                  </td>
                  <td className="py-1.5 pr-4 tabular-nums">{p.totals.messages}</td>
                  <td className="py-1.5 pr-4 tabular-nums">{p.totals.sessions}</td>
                  <td className="py-1.5 pr-4 tabular-nums">{formatTokens(tokens(p.totals))}</td>
                  <td className="py-1.5 tabular-nums">{formatTokens(p.totals.cache_read_tokens)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

/// All Claude use (usage tab spec section 6): the person's own Claude Code sessions and Dots',
/// from Claude Code's transcripts on this PC.
function AllClaude(props: { period: UsagePeriod; span: string; scanning: boolean }) {
  const data = useQuery({
    queryKey: keys.claudeUsage(props.period),
    queryFn: () => commands.claudeUsage(props.period),
  }).data;
  const id = `all-claude-${props.period}`;
  let status: string | null = null;
  if (props.scanning) status = "Reading Claude Code's transcripts…";
  else if (data?.available && data.scanned_at) {
    status = `Read ${relativeTime(data.scanned_at)} from ${data.folder}`;
  }
  return (
    <section aria-labelledby={id} className="flex flex-col gap-3 pt-4">
      <h2 id={id} className="font-medium">
        All Claude use
      </h2>
      <AllClaudeBody period={props.period} span={props.span} />
      <p className="text-xs text-muted-foreground" aria-live="polite">
        {status}
      </p>
    </section>
  );
}

/// The Usage page (usage and limits spec section 5): the account's Claude windows, then Dots'
/// own runs per dot over the last 5 hours, week or month, then all Claude use on this PC
/// (usage tab spec section 6).
export function UsageScreen() {
  const [period, setPeriod] = useState<UsagePeriod>("five_hours");
  const scanning = useClaudeScan();
  return (
    <Page title="Usage">
      <Limits />
      <Spending />
      <Tabs value={period} onValueChange={(v) => setPeriod(v as UsagePeriod)}>
        <TabsList aria-label="Period">
          {PERIODS.map((p) => (
            <TabsTrigger key={p.value} value={p.value}>
              {p.tab}
            </TabsTrigger>
          ))}
        </TabsList>
        {PERIODS.map((p) => (
          <TabsContent key={p.value} value={p.value} className="flex flex-col gap-3">
            <UsageTable period={p.value} span={p.span} />
            <p className="text-xs text-muted-foreground">
              A deleted dot's runs are deleted with it and not counted.
            </p>
            <AllClaude period={p.value} span={p.span} scanning={scanning} />
          </TabsContent>
        ))}
      </Tabs>
    </Page>
  );
}
