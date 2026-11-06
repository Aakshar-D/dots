import { Plus, X } from "lucide-react";
import { useState } from "react";

import type { AgentStep, BranchStep, CheckStep, DeployStep, Step, Team } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { plural } from "@/screens/library/parts";

import { LimitField } from "./cards";
import {
  agentsBefore,
  findStep,
  MAX_JOB,
  MAX_NAME,
  MAX_OUTCOME,
  NEW_DELEGATE,
  newPathLabel,
  outcomeSource,
  toggled,
  withMode,
} from "./draft";
import { ChoiceList, FolderWarning, Group, MemberSelect } from "./fields";
import {
  DEPLOY_LABEL,
  KIND_LABEL,
  MERGE_OPTIONS,
  MODE_OPTIONS,
  type NameOf,
  ON_OPTIONS,
} from "./labels";

/// What each kind's fields read: the step, the whole draft (for the steps before it), the team
/// and its members' names; `onChange` takes the whole changed step.
type KindProps<S extends Step> = {
  step: S;
  steps: Step[];
  team: Team;
  nameOf: NameOf;
  onChange: (step: Step) => void;
};

/// A step's name in a list of steps to choose from.
const nameIn = (steps: Step[], id: string) => findStep(steps, id)?.name.trim() || id;

/// The side sheet of a step's settings (section 10.2), open while `step` is set: its name, then
/// the fields of its kind. It edits the draft in place, so the live check sees each change; a
/// branch's paths hold steps of their own, which are edited on their cards.
export function StepSheet(props: {
  step: Step | undefined;
  steps: Step[];
  team: Team;
  nameOf: NameOf;
  onChange: (step: Step) => void;
  onClose: () => void;
}) {
  const { step } = props;
  return (
    <Sheet open={step !== undefined} onOpenChange={(open) => !open && props.onClose()}>
      {step && (
        <SheetContent>
          <SheetHeader>
            <SheetTitle>{KIND_LABEL[step.kind]} settings</SheetTitle>
            <SheetDescription>
              Changes go into the draft at once. Save keeps them.
            </SheetDescription>
          </SheetHeader>
          <Field label="Name" htmlFor="step-name" hint="Shown on the card and on the run page.">
            <Input
              id="step-name"
              maxLength={MAX_NAME}
              value={step.name}
              onChange={(e) => props.onChange({ ...step, name: e.target.value })}
            />
          </Field>
          <KindFields {...props} step={step} />
        </SheetContent>
      )}
    </Sheet>
  );
}

function KindFields(props: KindProps<Step>) {
  const { step } = props;
  switch (step.kind) {
    case "agent":
      return <AgentFields {...props} step={step} />;
    case "check":
      return <CheckFields {...props} step={step} />;
    case "deploy":
      return <DeployFields {...props} step={step} />;
    case "branch":
      return <BranchFields {...props} step={step} />;
  }
}

/// The job of a step's member: a textarea within the core's limit.
function JobField(props: { value: string; hint: string; onChange: (job: string) => void }) {
  return (
    <Field label="Job" htmlFor="step-job" hint={props.hint}>
      <Textarea
        id="step-job"
        maxLength={MAX_JOB}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
      />
    </Field>
  );
}

/// An agent step (sections 5.2 and 5.3): its member and job, the earlier agent steps it may
/// send work back to, the outcomes it may report, and whether it is a delegate.
function AgentFields({ step, steps, team, nameOf, onChange }: KindProps<AgentStep>) {
  const earlier = agentsBefore(steps, step.id);
  const d = step.delegate;
  return (
    <>
      <MemberSelect
        id="step-member"
        label="Member"
        hint="The dot of the team that does this step."
        members={team.members}
        value={step.member}
        nameOf={nameOf}
        onChange={(member) => onChange({ ...step, member })}
      />
      <FolderWarning dotIds={[step.member]} nameOf={nameOf} />
      <JobField
        value={step.job}
        hint="What this step does. It is added to the member's own instructions."
        onChange={(job) => onChange({ ...step, job })}
      />
      <ChoiceList
        legend="May send work back to"
        hint="Earlier agent steps this member may send the work back to, with a note."
        options={earlier.map((s) => ({ value: s.id, label: s.name.trim() || "(no name)" }))}
        chosen={step.can_send_back_to}
        stale={(id) => nameIn(steps, id)}
        empty="No agent step comes before it."
        onChange={(id, on) =>
          onChange({ ...step, can_send_back_to: toggled(step.can_send_back_to, id, on) })
        }
      />
      <OutcomeList
        outcomes={step.outcomes}
        onChange={(outcomes) => onChange({ ...step, outcomes })}
      />
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-2">
          <Switch
            id="step-delegate"
            checked={d !== null}
            onCheckedChange={(on) => onChange({ ...step, delegate: on ? NEW_DELEGATE : null })}
          />
          <Label htmlFor="step-delegate">It may start other members</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          A delegate step: its member starts members of the team on parts of the task, and the
          step ends when every one of them has ended.
        </p>
      </div>
      {d && (
        <>
          <ChoiceList
            legend="May start"
            options={team.members.map((m) => ({ value: m.dot_id, label: m.dot_name }))}
            chosen={d.may_start}
            stale={nameOf}
            empty="The team has no member yet."
            onChange={(id, on) =>
              onChange({ ...step, delegate: { ...d, may_start: toggled(d.may_start, id, on) } })
            }
          />
          <FolderWarning dotIds={d.may_start} nameOf={nameOf} />
          <div className="grid grid-cols-2 gap-3">
            <LimitField
              id="step-at-once"
              label="At once"
              value={d.at_once}
              onChange={(at_once) => onChange({ ...step, delegate: { ...d, at_once } })}
            />
            <LimitField
              id="step-total"
              label="Total"
              value={d.total}
              onChange={(total) => onChange({ ...step, delegate: { ...d, total } })}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            At once is a ceiling: the limit on runs at once in Settings still applies.
          </p>
        </>
      )}
    </>
  );
}

/// The labels an agent step may report: each with a remove button, and a field to add one.
function OutcomeList(props: { outcomes: string[]; onChange: (outcomes: string[]) => void }) {
  const [text, setText] = useState("");
  const label = text.trim();
  const twice = props.outcomes.some((o) => o.trim() === label);
  function add() {
    if (label === "" || twice) return;
    props.onChange([...props.outcomes, label]);
    setText("");
  }
  return (
    <Group
      legend="Outcomes"
      hint="Labels the member may report, for a branch on outcome after this step."
    >
      {props.outcomes.length > 0 && (
        <ul aria-label="Outcomes" className="flex flex-wrap gap-1.5">
          {props.outcomes.map((o, i) => (
            <li
              key={i}
              className="flex items-center gap-1 rounded-md border bg-card py-0.5 pr-0.5 pl-2 text-xs"
            >
              {o}
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Remove the outcome ${o}`}
                title="Remove"
                onClick={() => props.onChange(props.outcomes.filter((_, j) => j !== i))}
              >
                <X aria-hidden />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <Input
          aria-label="New outcome"
          placeholder="fixed"
          maxLength={MAX_OUTCOME}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            add();
          }}
        />
        <Button variant="outline" size="sm" disabled={label === "" || twice} onClick={add}>
          <Plus aria-hidden /> Add outcome
        </Button>
      </div>
      {label !== "" && twice && <p className="text-xs text-destructive">It is listed already.</p>}
    </Group>
  );
}

/// A check (section 5.4): the agent step a send-back goes to.
function CheckFields({ step, steps, onChange }: KindProps<CheckStep>) {
  const earlier = agentsBefore(steps, step.id);
  const to = step.send_back_to;
  // A target that is no longer before it stays shown until another is picked.
  const stale = to !== null && !earlier.some((s) => s.id === to) ? to : null;
  return (
    <Field
      label="Send back to"
      htmlFor="step-send-back-to"
      hint="Where Send back takes the work, with your note."
    >
      <NativeSelect
        id="step-send-back-to"
        className="w-full"
        value={to ?? ""}
        onChange={(e) => onChange({ ...step, send_back_to: e.target.value || null })}
      >
        <NativeSelectOption value="">The nearest agent step before it</NativeSelectOption>
        {stale !== null && (
          <NativeSelectOption value={stale}>
            {nameIn(steps, stale)} (not before it)
          </NativeSelectOption>
        )}
        {earlier.map((s) => (
          <NativeSelectOption key={s.id} value={s.id}>
            {s.name.trim() || "(no name)"}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Field>
  );
}

/// A deploy (section 5.5): its mode, and the mode's own fields.
function DeployFields({ step, team, nameOf, onChange }: KindProps<DeployStep>) {
  return (
    <>
      <Field label="Mode" htmlFor="step-mode" hint={DEPLOY_LABEL[step.mode]}>
        <NativeSelect
          id="step-mode"
          className="w-full"
          value={step.mode}
          onChange={(e) => {
            const mode = MODE_OPTIONS.find(([m]) => m === e.target.value)?.[0];
            if (mode) onChange(withMode(step, mode, team));
          }}
        >
          {MODE_OPTIONS.map(([mode, label]) => (
            <NativeSelectOption key={mode} value={mode}>
              {label}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      {step.mode === "pr_merge" && (
        <Field label="Merge method" htmlFor="step-merge-method">
          <NativeSelect
            id="step-merge-method"
            className="w-full"
            value={step.merge_method}
            onChange={(e) => {
              const method = MERGE_OPTIONS.find(([m]) => m === e.target.value)?.[0];
              if (method) onChange({ ...step, merge_method: method });
            }}
          >
            {MERGE_OPTIONS.map(([method, label]) => (
              <NativeSelectOption key={method} value={method}>
                {label}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
      )}
      {step.mode === "member" && (
        <>
          <MemberSelect
            id="step-member"
            label="Member"
            hint="The dot of the team that deploys what the check approved."
            members={team.members}
            value={step.member}
            nameOf={nameOf}
            onChange={(member) => onChange({ ...step, member })}
          />
          <FolderWarning dotIds={[step.member]} nameOf={nameOf} />
          <JobField
            value={step.job}
            hint="How it deploys, for example: tag a release."
            onChange={(job) => onChange({ ...step, job })}
          />
        </>
      )}
      <p className="text-xs text-muted-foreground">
        Git and gh run as you, with your own credentials. A Test run pushes nothing: each deploy
        keeps the branches.
      </p>
    </>
  );
}

/// A branch (section 5.6): what it goes by, its paths' labels, and what Otherwise holds.
function BranchFields({ step, steps, team, onChange }: KindProps<BranchStep>) {
  const source = step.on === "outcome" ? outcomeSource(steps, step.id) : undefined;
  const labels =
    step.on === "outcome" ? (source?.outcomes ?? []) : team.projects.map((p) => p.name);
  const listed = labels.join(", ");
  const hint =
    step.on === "project"
      ? `A path's label is a project's name: ${listed || "the team has no project yet"}.`
      : source
        ? `A path's label is an outcome of ${source.name.trim() || "(no name)"}: ` +
          `${listed || "it has none yet"}.`
        : "No agent step comes before it to report an outcome.";
  const setPaths = (paths: BranchStep["paths"]) => onChange({ ...step, paths });
  const inOtherwise = step.otherwise.length;
  return (
    <>
      <Field label="Goes by" htmlFor="step-on" hint={hint}>
        <NativeSelect
          id="step-on"
          className="w-full"
          value={step.on}
          onChange={(e) => {
            const on = ON_OPTIONS.find(([o]) => o === e.target.value)?.[0];
            if (on) onChange({ ...step, on });
          }}
        >
          {ON_OPTIONS.map(([on, label]) => (
            <NativeSelectOption key={on} value={on}>
              {label}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      <Group legend="Paths" hint="Each path's steps are its cards under the branch.">
        <ul className="flex flex-col gap-1.5">
          {step.paths.map((path, i) => (
            <li key={i} className="flex items-center gap-2">
              <Input
                aria-label={`Label of path ${i + 1}`}
                maxLength={step.on === "outcome" ? MAX_OUTCOME : undefined}
                value={path.label}
                onChange={(e) => {
                  const label = e.target.value;
                  setPaths(step.paths.map((p, j) => (j === i ? { ...p, label } : p)));
                }}
              />
              <RemovePath
                label={path.label.trim() || `path ${i + 1}`}
                inside={path.steps.length}
                onRemove={() => setPaths(step.paths.filter((_, j) => j !== i))}
              />
            </li>
          ))}
        </ul>
        <Button
          variant="outline"
          size="sm"
          className="w-fit"
          onClick={() =>
            setPaths([...step.paths, { label: newPathLabel(step, labels), steps: [] }])
          }
        >
          <Plus aria-hidden /> Add path
        </Button>
      </Group>
      <p className="text-xs text-muted-foreground">
        {inOtherwise === 0
          ? "Otherwise is empty: when no path's label matches, the workflow goes on after the " +
            "branch."
          : `Otherwise holds ${plural(inOtherwise, "step")}, run when no path's label matches.`}
      </p>
    </>
  );
}

/// A path's remove button. A path that holds steps asks first: they go with it.
function RemovePath(props: { label: string; inside: number; onRemove: () => void }) {
  const name = `Remove the path ${props.label}`;
  if (props.inside === 0) {
    return (
      <Button
        variant="ghost"
        size="icon-xs"
        aria-label={name}
        title="Remove"
        onClick={props.onRemove}
      >
        <X aria-hidden />
      </Button>
    );
  }
  return (
    <ConfirmButton
      variant="ghost"
      size="icon-xs"
      ariaLabel={name}
      title={`${name}?`}
      description={
        `${props.inside === 1 ? "The step on it goes" : `The ${props.inside} steps on it go`} ` +
        `with it. Nothing is saved until you press Save.`
      }
      confirmLabel="Remove path"
      onConfirm={props.onRemove}
    >
      <X aria-hidden />
    </ConfirmButton>
  );
}
