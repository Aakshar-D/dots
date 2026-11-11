import { useState, type ReactNode } from "react";

import type { Deed, OpenDotCard, Schedule, SetupProposal, Weekday } from "@/api/types";
import { localDateTime } from "@/lib/format";

const DAY_NAMES: Record<Weekday, string> = {
  mon: "Monday",
  tue: "Tuesday",
  wed: "Wednesday",
  thu: "Thursday",
  fri: "Friday",
  sat: "Saturday",
  sun: "Sunday",
};
const WEEK: Weekday[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// "Monday, Wednesday and Friday".
function joined(items: string[]): string {
  if (items.length <= 1) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

/// A schedule in a few plain words: "Weekdays at 09:00", "Every 2 hours", "Once, 12 Nov 2026,
/// 09:00". The app had no such sentence (its screens edit a schedule in fields), so it starts
/// here. A cron expression is shown as written.
export function scheduleWords(schedule: Schedule): string {
  const rule = schedule.rule;
  let words: string;
  switch (rule.kind) {
    case "once":
      return `Once, ${localDateTime(rule.at)}`;
    case "every": {
      const m = rule.minutes;
      if (m % 60 === 0) words = m === 60 ? "Every hour" : `Every ${m / 60} hours`;
      else words = m === 1 ? "Every minute" : `Every ${m} minutes`;
      break;
    }
    case "daily":
      words = `Every day at ${rule.time}`;
      break;
    case "weekly": {
      const days = WEEK.filter((d) => rule.days.includes(d));
      const key = days.join(",");
      const when =
        key === "mon,tue,wed,thu,fri"
          ? "Weekdays"
          : key === "sat,sun"
            ? "Weekends"
            : days.length === 7
              ? "Every day"
              : `Every ${joined(days.map((d) => DAY_NAMES[d]))}`;
      words = `${when} at ${rule.time}`;
      break;
    }
    case "cron":
      words = `On the schedule ${rule.expr}`;
      break;
  }
  const until = schedule.until;
  if (until && "count" in until) words += `, ${until.count} ${until.count === 1 ? "time" : "times"}`;
  else if (until && "date" in until) words += `, until ${until.date}`;
  return words;
}

/// One labelled part of a card.
function Part(props: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <p className="text-xs text-muted-foreground">{props.label}</p>
      {props.children}
    </div>
  );
}

/// Instructions this long are clamped with a way to read them all; shorter ones fit the clamp.
const LONG_INSTRUCTIONS = 280;

function ProposalCard({ proposal }: { proposal: SetupProposal }) {
  const [all, setAll] = useState(false);
  const long =
    proposal.instructions.length > LONG_INSTRUCTIONS || proposal.instructions.split("\n").length > 6;
  return (
    <div className="flex flex-col gap-2.5">
      <p className="text-sm font-medium">{proposal.name}</p>
      <Part label="What it does">
        <p className={all ? "whitespace-pre-wrap" : "line-clamp-6 whitespace-pre-wrap"}>
          {proposal.instructions}
        </p>
        {long && (
          <button
            type="button"
            aria-expanded={all}
            onClick={() => setAll((a) => !a)}
            className="self-start rounded-sm text-xs text-primary underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          >
            {all ? "Show less" : "Show all"}
          </button>
        )}
      </Part>
      <Part label="When it runs">
        <p>{proposal.schedule ? scheduleWords(proposal.schedule) : "Only when you talk to it"}</p>
      </Part>
      <Part label="Where it works">
        <p className="break-all">{proposal.folder ?? "A new folder"}</p>
      </Part>
      {proposal.first_message.trim() !== "" && (
        <Part label="First message">
          <p className="whitespace-pre-wrap">{proposal.first_message}</p>
        </Part>
      )}
    </div>
  );
}

/// What each deed is called on its card.
const DEED_TITLE: Record<Deed["kind"], string> = {
  approve: "Allow",
  deny: "Deny",
  run: "Run now",
  send: "Send",
  answer: "Answer",
};

function ConfirmCard({ deed }: { deed: Deed }) {
  return (
    <div className="flex flex-col gap-1.5">
      <p className="text-xs text-muted-foreground">{DEED_TITLE[deed.kind]}</p>
      <p className="text-sm font-medium">{deed.dot_name}</p>
      {(deed.kind === "approve" || deed.kind === "deny") && (
        <>
          <p>{deed.tool}</p>
          <code className="block rounded-md bg-background/60 px-2 py-1 font-mono text-xs whitespace-pre-wrap wrap-anywhere">
            {deed.target}
          </code>
        </>
      )}
      {(deed.kind === "send" || deed.kind === "answer") && (
        <p className="whitespace-pre-wrap">{deed.text}</p>
      )}
    </div>
  );
}

function ListCard({ title, lines }: { title: string; lines: string[] }) {
  return (
    <div className="flex flex-col gap-1.5">
      <p className="text-sm font-medium">{title}</p>
      <ul className="flex flex-col gap-1">
        {lines.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ul>
    </div>
  );
}

/// What Open Dot shows beside its words. Read-only: the choice is made with the options.
export function Card({ card }: { card: OpenDotCard }) {
  switch (card.kind) {
    case "proposal":
      return <ProposalCard proposal={card.proposal} />;
    case "confirm":
      return <ConfirmCard deed={card.deed} />;
    case "list":
      return <ListCard title={card.title} lines={card.lines} />;
  }
}
