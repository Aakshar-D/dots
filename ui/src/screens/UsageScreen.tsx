import { useQuery } from "@tanstack/react-query";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Fragment, useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { LimitReading, UsagePeriod, UsageRow } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { localDateTime } from "@/lib/format";
import { triggerLabel } from "@/lib/home-topics";
import { keys } from "@/lib/query";
import { formatCost, formatDuration, formatTokens, pct } from "@/lib/usage-format";

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

/// The account's windows (the whole account, not only Dots' runs) and the held runs.
function Limits() {
  const limits = useQuery({ queryKey: keys.limits, queryFn: commands.limitsGet }).data;
  if (!limits) return null;
  const held = limits.held_run_ids.length;
  return (
    <section aria-label="Claude usage" className="flex flex-wrap items-center gap-x-6 gap-y-2">
      <Window label="5-hour" reading={limits.five_hour} share={limits.shares.five_hour} />
      <Window label="weekly" reading={limits.weekly} share={limits.shares.weekly} />
      {held > 0 && (
        <Link to="/inbox" className="text-sm underline underline-offset-4">
          Holding {held} runs
        </Link>
      )}
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
  return (
    <div className="max-w-full overflow-x-auto">
      <table className="w-full text-left text-sm">
        <caption className="sr-only">Dots' runs in the last {props.span}</caption>
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

/// The Usage page (usage and limits spec section 5): the account's Claude windows, then Dots'
/// own runs per dot over the last 5 hours, week or month.
export function UsageScreen() {
  const [period, setPeriod] = useState<UsagePeriod>("five_hours");
  return (
    <Page title="Usage">
      <Limits />
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
          </TabsContent>
        ))}
      </Tabs>
    </Page>
  );
}
