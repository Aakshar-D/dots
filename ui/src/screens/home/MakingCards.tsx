import { useQuery } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { MakingCard, Run } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { MakeRequestCard, Written } from "@/components/MakeRequestCard";
import { Button } from "@/components/ui/button";
import { cardsAfter, jobEndWords } from "@/lib/making";
import { keys } from "@/lib/query";

/// The making cards of the topic whose root run is `rootRunId` (dots-make-dots design section
/// 11), oldest first. Every topic asks: a sol whose switch was turned off keeps its cards. A
/// topic no maker worked in has none.
export function useMakingCards(rootRunId: string | undefined) {
  return useQuery({
    queryKey: keys.makingCards(rootRunId ?? ""),
    queryFn: () => commands.makingCards(rootRunId ?? ""),
    enabled: rootRunId !== undefined,
  });
}

function Shell(props: { label: string; children: ReactNode }) {
  return (
    <article aria-label={props.label} className="flex flex-col gap-1 rounded-lg border bg-card p-3">
      {props.children}
    </article>
  );
}

function OpenRun({ card }: { card: Extract<MakingCard, { job_id: string }> }) {
  if (card.job_run_id === null || !card.run_exists) return null;
  return (
    <Button asChild size="sm" variant="outline" className="self-start">
      <Link to={`/runs/${card.job_run_id}`}>Open run</Link>
    </Button>
  );
}

/// One thing a maker did in its chat topic. Names, purposes and job texts were written by a
/// sol: they are drawn as plain text, and every link goes to an id the core gave.
function MakingCardView({ card }: { card: MakingCard }) {
  const name = card.dot_name;
  switch (card.kind) {
    case "made": {
      const title = `Made sol: ${name}`;
      return (
        <Shell label={title}>
          <p className="font-semibold">
            {title}{" "}
            <span className="text-xs font-normal text-muted-foreground">
              · made by {card.maker_name}
            </span>
          </p>
          {card.purpose !== "" && <p className="text-sm break-words">{card.purpose}</p>}
          {card.schedule !== null && (
            <p className="text-xs text-muted-foreground">Schedule: {card.schedule}</p>
          )}
          {card.uses !== null && (
            <p className="text-xs break-words text-muted-foreground">{card.uses}</p>
          )}
          {card.dot_id !== null ? (
            <Button asChild size="sm" variant="outline" className="self-start">
              <Link to={`/dots/${card.dot_id}`}>Open sol</Link>
            </Button>
          ) : (
            <p className="text-xs text-muted-foreground">This sol has been deleted since.</p>
          )}
        </Shell>
      );
    }
    case "asked":
      return card.card !== null ? (
        <MakeRequestCard card={card.card} />
      ) : (
        <Shell label={`${card.maker_name} asked about ${name}`}>
          <p className="font-semibold">
            {card.maker_name} asked about {name}
          </p>
          <p className="text-sm text-muted-foreground">
            This request is gone: the sol it was for was deleted.
          </p>
        </Shell>
      );
    case "changed": {
      const title = `${card.maker_name} changed ${name}`;
      return (
        <Shell label={title}>
          <p className="font-semibold">{title}</p>
          {card.what.length > 0 && (
            <ul className="list-disc pl-5 text-sm">
              {card.what.map((line, i) => (
                <li key={`${i}:${line}`} className="break-words">
                  {line}
                </li>
              ))}
            </ul>
          )}
        </Shell>
      );
    }
    case "deleted": {
      const title = `${card.maker_name} deleted ${name}`;
      return (
        <Shell label={title}>
          <p className="font-semibold">{title}</p>
        </Shell>
      );
    }
    case "job_started": {
      // The card of a job that has moved on no longer says the sol is working.
      const title = card.status === "working" ? `${name} is working` : `${name} was given a job`;
      return (
        <Shell label={title}>
          <p className="font-semibold">{title}</p>
          <Written label={`Job from ${card.maker_name}`} by={card.maker_name}>
            {card.message}
          </Written>
          <OpenRun card={card} />
        </Shell>
      );
    }
    case "job_waiting": {
      const title =
        card.status === "waiting_answer"
          ? `${name} is waiting for your answer`
          : `${name} asked you a question`;
      return (
        <Shell label={title}>
          <p className="font-semibold">{title}</p>
          <OpenRun card={card} />
        </Shell>
      );
    }
    case "job_ended": {
      const title = `${name} ${jobEndWords(card.how)}`;
      return (
        <Shell label={title}>
          <p className="font-semibold">{title}</p>
          <p className="text-xs text-muted-foreground">
            Sage tells {card.maker_name} how it ended.
          </p>
          <OpenRun card={card} />
        </Shell>
      );
    }
  }
}

/// The cards, in the order things happened.
export function MakingCards({ cards }: { cards: MakingCard[] }) {
  return cards.map((card) => <MakingCardView key={card.id} card={card} />);
}

/// The making cards of the first run of a run topic (one a schedule, a webhook or Run now
/// started), under its card; the runs that continued it draw their own (`TopicRuns`).
export function RootMakingCards({ runs }: { runs: Run[] }) {
  const root = runs[0];
  const making = useMakingCards(root?.root_run_id);
  if (root === undefined) return null;
  if (making.isError) return <ErrorNote error={making.error} />;
  return <MakingCards cards={cardsAfter(making.data ?? [], root, runs)} />;
}
