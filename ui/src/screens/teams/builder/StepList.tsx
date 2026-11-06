import { cn } from "cn";
import { ArrowDown, ChevronDown, ChevronUp, Plus, Settings2, Trash2 } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { createContext, useContext, useRef, type KeyboardEvent, type MouseEvent } from "react";

import type { Step, WorkflowProblem } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

import {
  allSteps,
  canMove,
  childLists,
  type ListRef,
  STEP_KINDS,
  type StepKindName,
} from "./draft";
import { delegateLine, KIND_LABEL, type NameOf, stepLine } from "./labels";

/// What the step cards read and do, from the builder that holds the draft.
export type Builder = {
  /// The whole draft's steps.
  steps: Step[];
  nameOf: NameOf;
  /// The live check's problem when it names a step of the draft.
  problem: WorkflowProblem | null;
  add: (list: ListRef, index: number, kind: StepKindName) => void;
  move: (id: string, by: -1 | 1) => void;
  remove: (id: string) => void;
  /// Opens the step's settings in the side sheet.
  open: (id: string) => void;
  /// Keeps each card's element, so focus can follow a card that moved.
  card: (id: string, el: HTMLElement | null) => void;
};

const BuilderContext = createContext<Builder | null>(null);
export const BuilderProvider = BuilderContext.Provider;

function useBuilder(): Builder {
  const builder = useContext(BuilderContext);
  if (!builder) throw new Error("a step card needs the builder");
  return builder;
}

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[highlighted]:bg-accent";

/// "＋ Add step": a menu of the kinds of step (section 5) that puts the new one into `list` at
/// `index`. With `text` the button says "Add step"; without, it is a small ＋ between two cards.
export function AddStep(props: { list: ListRef; index: number; label: string; text?: boolean }) {
  const { add } = useBuilder();
  // A chosen kind puts the focus on the new card, so the closing menu must not take it back.
  const chosen = useRef(false);
  return (
    <DropdownMenu.Root modal={false}>
      <DropdownMenu.Trigger asChild>
        <Button
          variant={props.text ? "outline" : "ghost"}
          size={props.text ? "xs" : "icon-xs"}
          aria-label={props.label}
          title={props.text ? undefined : props.label}
        >
          <Plus aria-hidden />
          {props.text && "Add step"}
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="center"
          className="z-50 w-40 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
          onCloseAutoFocus={(e) => {
            if (chosen.current) e.preventDefault();
            chosen.current = false;
          }}
        >
          {STEP_KINDS.map((kind) => (
            <DropdownMenu.Item
              key={kind}
              className={ITEM}
              onSelect={() => {
                chosen.current = true;
                add(props.list, props.index, kind);
              }}
            >
              {KIND_LABEL[kind]}
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/// The arrow between two cards, with an Add step between them when `add` is given.
function Connector(props: { add?: { list: ListRef; index: number; label: string } }) {
  return (
    <div className="flex flex-col items-center py-1 text-muted-foreground">
      <ArrowDown aria-hidden className="size-4" />
      {props.add && <AddStep {...props.add} />}
    </div>
  );
}

/// A list of step cards in run order, numbered from `prefix`, joined by arrows, with Add step
/// between the cards and at the end. `name` names the list for a screen reader and the Add
/// step buttons ("the path fixed"). The top list also gets an Add step before its first card,
/// which follows the trigger card.
export function StepList(props: {
  steps: Step[];
  list: ListRef;
  prefix: string;
  name: string;
  top?: boolean;
}) {
  const { steps, list } = props;
  const endLabel = props.top ? "Add step" : `Add step to ${props.name}`;
  return (
    <div className="flex w-full flex-col items-stretch">
      {props.top && steps.length > 0 && (
        <Connector add={{ list, index: 0, label: `Add step before ${steps[0]!.name}` }} />
      )}
      <ol aria-label={props.top ? "Steps" : `Steps of ${props.name}`} className="flex flex-col">
        {steps.map((step, i) => (
          <li key={step.id} className="flex flex-col">
            {i > 0 && (
              <Connector add={{ list, index: i, label: `Add step after ${steps[i - 1]!.name}` }} />
            )}
            <StepCard step={step} number={`${props.prefix}${i + 1}`} />
          </li>
        ))}
      </ol>
      <div className="flex flex-col items-center gap-1 pt-1">
        {steps.length > 0 && <ArrowDown aria-hidden className="size-4 text-muted-foreground" />}
        <AddStep list={list} index={steps.length} label={endLabel} text />
      </div>
    </div>
  );
}

/// How a card of each kind looks (section 10.2): a delegate step dashed in the accent, a check
/// in the needs-you colours.
function look(step: Step): string {
  if (step.kind === "agent" && step.delegate) return "border-dashed border-primary";
  if (step.kind === "check") return "border-needs-border bg-needs-surface";
  return "";
}

/// Whether a click or key on the card is the card's own, not one of a control in it or of a
/// card inside it (a branch holds cards).
function own(e: MouseEvent | KeyboardEvent): boolean {
  const target = e.target as Element;
  if (target.closest("[data-step-id]") !== e.currentTarget) return false;
  return target === e.currentTarget || !target.closest("button, a, input, textarea, select");
}

/// One step as a card: its number, name and what it does, its kind's look, the problem the live
/// check found in it, and its buttons (settings, move up and down, delete). A click on it or
/// Enter opens its settings; Alt+ArrowUp and Alt+ArrowDown move it within its list. A branch
/// shows its paths as labelled columns of their own cards.
export function StepCard({ step, number }: { step: Step; number: string }) {
  const builder = useBuilder();
  const { nameOf, problem } = builder;
  const name = step.name.trim() || "(no name)";
  const mine = problem?.step_id === step.id ? problem : null;
  const problemId = `problem-${step.id}`;
  const delegate = step.kind === "agent" ? delegateLine(step, nameOf) : undefined;
  const badge = delegate ? "Delegate" : KIND_LABEL[step.kind];

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    // A card inside a branch handles its own keys first: the branch does not move with them.
    if (e.defaultPrevented) return;
    if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      builder.move(step.id, e.key === "ArrowUp" ? -1 : 1);
    } else if (e.key === "Enter" && e.target === e.currentTarget) {
      e.preventDefault();
      builder.open(step.id);
    }
  }

  return (
    <div
      ref={(el) => builder.card(step.id, el)}
      role="group"
      aria-label={`Step ${number}: ${name}`}
      aria-describedby={mine ? problemId : undefined}
      aria-invalid={mine ? true : undefined}
      tabIndex={0}
      data-step-id={step.id}
      data-kind={step.kind}
      onKeyDown={onKeyDown}
      onClick={(e) => own(e) && builder.open(step.id)}
      className={cn(
        "flex w-full flex-col gap-1 rounded-xl border bg-card p-3 text-left outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
        look(step),
        mine && "border-destructive",
      )}
    >
      <div className="flex items-start gap-2">
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className={cn("font-semibold", step.kind === "check" && "text-needs")}>
            {number} · {name}
          </span>
          <span className="text-xs text-muted-foreground">
            {stepLine(step, builder.steps, nameOf)}
          </span>
          {delegate && <span className="text-xs text-primary">{delegate}</span>}
        </div>
        <Badge variant="outline">{badge}</Badge>
        <StepButtons step={step} name={name} />
      </div>
      {mine && (
        <p id={problemId} className="text-xs text-destructive">
          {mine.message}
        </p>
      )}
      {step.kind === "branch" && (
        <div className="mt-2 flex gap-3 overflow-x-auto pl-4">
          {step.paths.map((path, i) => (
            <PathColumn
              key={i}
              title={path.label.trim() || "(no label)"}
              name={`the path ${path.label.trim()}`}
              region={`Path ${path.label.trim()}`}
              steps={path.steps}
              list={{ branch: step.id, path: i }}
              prefix={`${number}.`}
            />
          ))}
          <PathColumn
            title="Otherwise"
            name="Otherwise"
            region="Otherwise"
            steps={step.otherwise}
            list={{ branch: step.id, path: "otherwise" }}
            prefix={`${number}.`}
          />
        </div>
      )}
    </div>
  );
}

/// One path of a branch: its label over its own list of cards.
function PathColumn(props: {
  title: string;
  name: string;
  region: string;
  steps: Step[];
  list: ListRef;
  prefix: string;
}) {
  return (
    <section
      aria-label={props.region}
      className="flex min-w-56 flex-1 flex-col gap-1 rounded-lg border border-dashed p-2"
    >
      <h3 className="text-xs font-medium text-muted-foreground">{props.title}</h3>
      <StepList steps={props.steps} list={props.list} prefix={props.prefix} name={props.name} />
    </section>
  );
}

/// A card's settings, move and delete buttons. Deleting a branch that holds steps asks first:
/// they go with it. Any other step goes at once; the draft is not saved until Save.
function StepButtons({ step, name }: { step: Step; name: string }) {
  const builder = useBuilder();
  const inside = childLists(step).flatMap(allSteps).length;
  return (
    <div className="flex shrink-0 items-center">
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Settings of ${name}`}
        title="Settings"
        onClick={() => builder.open(step.id)}
      >
        <Settings2 aria-hidden />
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Move ${name} up`}
        title="Move up (Alt+Up)"
        disabled={!canMove(builder.steps, step.id, -1)}
        onClick={() => builder.move(step.id, -1)}
      >
        <ChevronUp aria-hidden />
      </Button>
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={`Move ${name} down`}
        title="Move down (Alt+Down)"
        disabled={!canMove(builder.steps, step.id, 1)}
        onClick={() => builder.move(step.id, 1)}
      >
        <ChevronDown aria-hidden />
      </Button>
      {inside > 0 ? (
        <ConfirmButton
          variant="ghost"
          size="icon-xs"
          ariaLabel={`Delete ${name}`}
          title={`Delete ${name}?`}
          description={
            `${inside === 1 ? "The step inside it goes" : `The ${inside} steps inside it go`} ` +
            `with it. Nothing is saved until you press Save.`
          }
          confirmLabel="Delete step"
          onConfirm={() => builder.remove(step.id)}
        >
          <Trash2 aria-hidden />
        </ConfirmButton>
      ) : (
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={`Delete ${name}`}
          title="Delete"
          onClick={() => builder.remove(step.id)}
        >
          <Trash2 aria-hidden />
        </Button>
      )}
    </div>
  );
}
