import { useMutation, useQueryClient } from "@tanstack/react-query";
import { cn } from "cn";
import { useState } from "react";
import { Link } from "react-router";

import { CommandError, commands } from "@/api/commands";
import type { MakeDecision, MakeRequestCard as Card, RequestKind } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { relativeTime } from "@/lib/format";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

/// The three answers, worded by what is asked (the core's refusals name them the same way).
const BUTTONS: Record<RequestKind, { allow: string; without: string; refuse: string }> = {
  make: { allow: "Allow for this sol", without: "Make it without these", refuse: "Don't make it" },
  change: {
    allow: "Allow for this sol",
    without: "Change it without these",
    refuse: "Don't change it",
  },
  // A job has no part to give without the rest.
  job: { allow: "Allow the job", without: "", refuse: "Don't give it" },
};

/// What became of an answered request, in one sentence.
function outcome(card: Card): string | null {
  const { kind, status } = card.request;
  switch (status) {
    case "allowed":
      return kind === "make"
        ? "You allowed it: the sol was made as asked."
        : kind === "change"
          ? "You allowed it: the sol was changed as asked."
          : "You allowed it: the job was given.";
    case "made_without":
      return kind === "change"
        ? "You chose to change it without what was beyond the maker's reach."
        : "You chose to make it without what was beyond the maker's reach.";
    case "refused":
      return kind === "make"
        ? "You chose not to make it."
        : kind === "change"
          ? "You chose not to change it."
          : "You chose not to give the job.";
    case "discarded":
      return "This request was dropped without an answer.";
    case "waiting":
    case "gone":
      return null;
  }
}

/// Everything the card says that an answer covers, as one string: when it differs from what
/// the card said when it was first shown, the person is told before they answer.
function said(card: Card): string {
  const { request } = card;
  return JSON.stringify([
    card.title,
    card.name,
    request.purpose,
    card.lines,
    card.asked,
    card.instructions,
    card.schedule,
    card.limit,
    card.shown,
    request.first_job,
    request.kind === "make" ? request.spec.model : null,
  ]);
}

/// Text a sol wrote (instructions, a job, a model's name), set apart from Sage's own words and
/// drawn as plain text: never as Markdown, never as a link.
export function Written(props: { label: string; by: string; children: string }) {
  return (
    <figure aria-label={props.label} className="flex flex-col gap-1">
      <figcaption className="text-xs text-muted-foreground">
        {props.label} · written by {props.by}, not by Sage
      </figcaption>
      <pre className="max-h-60 overflow-auto rounded-md border border-dashed bg-background/60 p-2 font-sans text-xs break-words whitespace-pre-wrap">
        {props.children}
      </pre>
    </figure>
  );
}

function Lines(props: { label: string; lines: string[] }) {
  if (props.lines.length === 0) return null;
  return (
    <section aria-label={props.label} className="flex flex-col gap-0.5">
      <h3 className="text-xs text-muted-foreground">{props.label}</h3>
      <ul className="list-disc pl-5 text-sm">
        {props.lines.map((line, i) => (
          <li key={`${i}:${line}`} className="break-words">
            {line}
          </li>
        ))}
      </ul>
    </section>
  );
}

/// A maker's request that waits for the person, or was answered (dots-make-dots design section
/// 7.2): what is asked, what of it lies beyond the maker's own reach, and the three answers.
/// The core builds the card and says which buttons are on (`can_allow`, `can_without`) and
/// why Allow is off (`blocked`); this draws it and sends the answer.
///
/// An answer goes with the `limit` and `shown` of the card as it was read, so it covers only
/// what the card showed. When the core refuses it because the card no longer says what the
/// request would give (`conflict`), the card is read again and says so before Allow is on
/// again.
export function MakeRequestCard({ card: given }: { card: Card }) {
  const qc = useQueryClient();
  // The card read again after an answer or a refusal. It stands in for the one given until the
  // list it came from brings a newer one.
  const [reread, setReread] = useState<{ card: Card; over: Card } | null>(null);
  const card = reread !== null && reread.over === given ? reread.card : given;
  const { request } = card;
  // What the card said when the person first saw it.
  const [first] = useState(() => said(given));
  // The core's words when it refused an answer because the card had changed.
  const [changed, setChanged] = useState<string | null>(null);
  // The card could not be read again after such a refusal: nothing is allowed from what is
  // shown until the list brings the card as it is now.
  const [staleFor, setStaleFor] = useState<Card | null>(null);
  const [rereadError, setRereadError] = useState<unknown>(null);
  // The request went with its maker, or with the made sol it was for.
  const [gone, setGone] = useState(false);
  const [rereading, setRereading] = useState(false);

  const refresh = () => {
    for (const queryKey of [
      keys.makingCards(request.root_run_id),
      keys.makeRequestsWaiting,
      keys.inbox,
      keys.overview,
    ]) {
      invalidate(qc, queryKey);
    }
  };
  const decide = useMutation({
    mutationFn: (answer: { decision: MakeDecision; limit: string | null; shown: string | null }) =>
      commands.makeRequestDecide(request.id, answer.decision, answer.limit, answer.shown),
    onSuccess: (after, answer) => {
      setChanged(null);
      setReread({
        card: { ...card, request: after, can_allow: false, can_without: false, blocked: null },
        over: given,
      });
      refresh();
      // A sol was made or changed, and no dot event says so.
      if (answer.decision !== "refuse") invalidateDots(qc);
    },
    onError: (error) => {
      if (!(error instanceof CommandError) || error.kind !== "conflict") return;
      // What the card showed is not what the request would give now, or it has its answer:
      // the card is read again, and only then can it be answered again.
      setChanged(error.message);
      setRereading(true);
      setRereadError(null);
      commands
        .makeRequestCard(request.id)
        .then(
          (fresh) => setReread({ card: fresh, over: given }),
          (e: unknown) => {
            if (e instanceof CommandError && e.kind === "not_found") setGone(true);
            else {
              setStaleFor(given);
              setRereadError(e);
            }
          },
        )
        .finally(() => {
          setRereading(false);
          refresh();
        });
    },
  });

  const buttons = BUTTONS[request.kind];
  const waiting = request.status === "waiting";
  // A request whose named things have gone can still be refused, which tells the maker.
  const open = waiting || request.status === "gone";
  const done = outcome(card);
  if (gone) {
    return (
      <article aria-label={card.title} className="flex flex-col gap-1 rounded-xl border bg-card p-3">
        <p className="font-semibold">{card.title}</p>
        <p className="text-sm text-muted-foreground">
          This request is gone: its maker, or the sol it was for, was deleted.
        </p>
      </article>
    );
  }
  const stale = staleFor === given;
  const busy = decide.isPending || decide.isSuccess || rereading;
  const moved = waiting && (changed !== null || said(card) !== first);
  // For a job the core's last line says where the made sol's final message goes.
  const isJob = request.kind === "job";
  const beyond = isJob ? card.lines.slice(0, -1) : card.lines;
  const resultLine = isJob ? card.lines.at(-1) : undefined;
  const send = (decision: MakeDecision) =>
    onSingleClick(() => decide.mutate({ decision, limit: card.limit, shown: card.shown }));
  const conflict = decide.error instanceof CommandError && decide.error.kind === "conflict";
  return (
    <article
      aria-label={card.title}
      className={cn(
        "flex flex-col gap-2 rounded-xl border p-3",
        open ? "border-needs-border bg-needs-surface" : "bg-card",
      )}
    >
      <header className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-col">
          <p className={cn("font-semibold", open && "text-needs")}>{card.title}</p>
          <p className="text-sm break-words">
            <span className="font-medium">{card.name}</span>
            {request.purpose !== "" && <> · {request.purpose}</>}
          </p>
          <p className="text-xs text-muted-foreground">
            Asked by{" "}
            <Link to={`/dots/${request.maker_dot_id}`} className="text-foreground hover:underline">
              {card.maker_name}
            </Link>{" "}
            {relativeTime(request.created_at)}
          </p>
        </div>
      </header>
      {moved && (
        <p role="alert" className="rounded-md border border-needs-border p-2 text-sm">
          {changed ?? "This request changed since you first saw it."} Read the card again before
          you answer.
        </p>
      )}
      <Lines
        label={
          isJob
            ? `What ${card.name} has that ${card.maker_name} does not`
            : card.shown === null
              ? "The limit it passes"
              : `Beyond what ${card.maker_name} has`
        }
        lines={beyond}
      />
      <Lines label="What the change sets" lines={card.asked} />
      {card.schedule !== null && (
        <p className="text-sm">
          <span className="text-xs text-muted-foreground">Schedule: </span>
          {card.schedule}
        </p>
      )}
      {request.kind === "make" && (
        <Written label="Model" by={card.maker_name}>
          {request.spec.model}
        </Written>
      )}
      {card.instructions !== null && (
        <Written
          label={request.kind === "change" ? "New instructions" : "Instructions"}
          by={card.maker_name}
        >
          {card.instructions}
        </Written>
      )}
      {request.first_job !== null && (
        <Written label={isJob ? `Job from ${card.maker_name}` : "First job"} by={card.maker_name}>
          {request.first_job}
        </Written>
      )}
      {resultLine !== undefined && <p className="text-sm">{resultLine}</p>}
      {done !== null && <p className="text-sm">{done}</p>}
      {done !== null && request.made_dot_id !== null && (
        <Button asChild size="sm" variant="outline" className="self-start">
          <Link to={`/dots/${request.made_dot_id}`}>Open sol</Link>
        </Button>
      )}
      {open && card.blocked !== null && (
        <p aria-label="Why Allow is off" className="text-xs text-muted-foreground">
          {card.blocked}
        </p>
      )}
      {decide.isError && !conflict && <ErrorNote error={decide.error} />}
      {rereadError !== null && stale && (
        <ErrorNote
          error={`The card could not be read again, so it can't be allowed from here yet: ${
            rereadError instanceof Error ? rereadError.message : String(rereadError)
          }`}
        />
      )}
      {open && (
        <div className="flex flex-wrap gap-2">
          {waiting && (
            <Button
              size="sm"
              disabled={busy || stale || !card.can_allow}
              title={card.can_allow ? undefined : (card.blocked ?? undefined)}
              onClick={send("allow")}
            >
              {buttons.allow}
            </Button>
          )}
          {waiting && !isJob && card.can_without && (
            <Button
              size="sm"
              variant="outline"
              disabled={busy || stale}
              onClick={send("without")}
            >
              {buttons.without}
            </Button>
          )}
          <Button size="sm" variant="outline" disabled={busy} onClick={send("refuse")}>
            {buttons.refuse}
          </Button>
        </div>
      )}
    </article>
  );
}
