import { Plus, X } from "lucide-react";
import { useState } from "react";

import type {
  ActionStep,
  AgentStep,
  BranchStep,
  CheckStep,
  DeployStep,
  HttpMethod,
  InputField,
  ModelTier,
  Step,
  StepOptions,
  Team,
  WaitStep,
  Weekday,
} from "@/api/types";
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
  agentsEarlier,
  findStep,
  marksFor,
  MAX_JOB,
  MAX_NAME,
  MAX_OUTCOME,
  MAX_OUTPUTS,
  nameOk,
  NEW_DELEGATE,
  newPathLabel,
  outcomeSource,
  toggled,
  withAction,
  withMode,
  withWait,
} from "./draft";
import { ChoiceList, FolderWarning, Group, MemberSelect } from "./fields";
import {
  DEPLOY_LABEL,
  KIND_LABEL,
  MERGE_OPTIONS,
  MODE_OPTIONS,
  MODEL_SIZE_OPTIONS,
  WEEKDAY_OPTIONS,
  type NameOf,
  ON_OPTIONS,
} from "./labels";

/// What each kind's fields read: the step, the whole draft (for the steps before it), the team
/// and its members' names; `onChange` takes the whole changed step.
type KindProps<S extends Step> = {
  step: S;
  steps: Step[];
  /// What a run of the workflow is started with: an action's texts and a wait may name them.
  inputs: InputField[];
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
  inputs: InputField[];
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
    case "action":
      return <ActionFields {...props} step={step} />;
    case "wait":
      return <WaitFields {...props} step={step} />;
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
        hint="The sol of the team that does this step."
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
      <OutputList
        stepId={step.id}
        outputs={step.produces}
        onChange={(produces) => onChange({ ...step, produces })}
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
      <UsageFields
        options={step.options}
        earlier={agentsEarlier(steps, step.id)}
        nameOfStep={(id) => nameIn(steps, id)}
        onChange={(options) => onChange({ ...step, options })}
      />
    </>
  );
}

/// What makes an agent step's run use less (lean workflows spec section 2): the size of its
/// model, a prompt without the member's memories and project map, and which earlier steps'
/// summaries it reads.
function UsageFields(props: {
  options: StepOptions;
  earlier: AgentStep[];
  nameOfStep: (id: string) => string;
  onChange: (options: StepOptions) => void;
}) {
  const { options } = props;
  const picks = options.reads !== null;
  return (
    <>
      <Field
        label="Model size"
        htmlFor="step-model"
        hint="For a member on Claude. The sizes are the Auto models in Settings."
      >
        <NativeSelect
          id="step-model"
          className="w-full"
          value={options.model ?? ""}
          onChange={(e) =>
            props.onChange({
              ...options,
              model: e.target.value === "" ? null : (e.target.value as ModelTier),
            })
          }
        >
          <NativeSelectOption value="">The member's own model</NativeSelectOption>
          {MODEL_SIZE_OPTIONS.map((o) => (
            <NativeSelectOption key={o.value} value={o.value}>
              {o.label}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-2">
          <Switch
            id="step-lean"
            checked={options.lean}
            onCheckedChange={(lean) => props.onChange({ ...options, lean })}
          />
          <Label htmlFor="step-lean">Lean prompt</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          The step gets the member's instructions, the task and its job, without the member's
          memories or the project map, and saves no memory. Less usage for a step that needs
          neither.
        </p>
      </div>
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center gap-2">
          <Switch
            id="step-reads"
            checked={picks}
            onCheckedChange={(on) => props.onChange({ ...options, reads: on ? [] : null })}
          />
          <Label htmlFor="step-reads">Pick the earlier steps it reads</Label>
        </div>
        <p className="text-xs text-muted-foreground">
          {picks
            ? "It gets the summaries of the steps ticked below, each whole, and no other."
            : "It gets every earlier step's summary: the one just before it whole, older ones shortened."}
        </p>
      </div>
      {options.reads !== null && (
        <ChoiceList
          legend="Reads"
          options={props.earlier.map((s) => ({ value: s.id, label: s.name.trim() || "(no name)" }))}
          chosen={options.reads}
          stale={props.nameOfStep}
          empty="No agent step comes before it."
          onChange={(id, on) =>
            props.onChange({ ...options, reads: toggled(options.reads ?? [], id, on) })
          }
        />
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

/// The outputs an agent step gives (general workflows spec section 5): each a name, shown as a
/// later Send or Save step writes it.
function OutputList(props: {
  stepId: string;
  outputs: string[];
  onChange: (outputs: string[]) => void;
}) {
  const [text, setText] = useState("");
  const name = text.trim();
  const twice = props.outputs.includes(name);
  const bad = name !== "" && (!nameOk(name) || name === "summary");
  const full = props.outputs.length >= MAX_OUTPUTS;
  function add() {
    if (name === "" || twice || bad || full) return;
    props.onChange([...props.outputs, name]);
    setText("");
  }
  return (
    <Group
      legend="Outputs"
      hint="Texts this step gives by name, such as a post or a subject line. Its reply ends with each one in a block, you can change them at a check, and a later Send or Save step uses them as written here."
    >
      {props.outputs.length > 0 && (
        <ul aria-label="Outputs" className="flex flex-wrap gap-1.5">
          {props.outputs.map((o, i) => (
            <li
              key={i}
              className="flex items-center gap-1 rounded-md border bg-card py-0.5 pr-0.5 pl-2 font-mono text-xs"
            >
              {`{{${props.stepId}.${o}}}`}
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={`Remove the output ${o}`}
                title="Remove"
                onClick={() => props.onChange(props.outputs.filter((_, j) => j !== i))}
              >
                <X aria-hidden />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <Input
          aria-label="New output"
          placeholder="post"
          maxLength={40}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter") return;
            e.preventDefault();
            add();
          }}
        />
        <Button
          variant="outline"
          size="sm"
          disabled={name === "" || twice || bad || full}
          onClick={add}
        >
          <Plus aria-hidden /> Add output
        </Button>
      </div>
      {twice && <p className="text-xs text-destructive">It is listed already.</p>}
      {bad && (
        <p className="text-xs text-destructive">
          Use a-z, 0-9, - and _ (not "summary", which every step has).
        </p>
      )}
    </Group>
  );
}

/// What a text of an action may hold, each as it is written.
function Marks(props: { marks: string[] }) {
  return (
    <Group
      legend="What you can put in"
      hint="Write any of these where its text should go. Each is filled in when the step runs."
    >
      <ul aria-label="What you can put in" className="flex flex-wrap gap-1.5">
        {props.marks.map((mark) => (
          <li key={mark} className="rounded-md border bg-card px-2 py-0.5 font-mono text-xs">
            {mark}
          </li>
        ))}
      </ul>
    </Group>
  );
}

const ACTION_OPTIONS: [ActionStep["action"], string][] = [
  ["http", "Send a request to a web address"],
  ["file", "Save a text file"],
];

const METHODS: HttpMethod[] = ["post", "get", "put", "patch", "delete"];

/// An action (general workflows spec section 6): what Sage itself does, with no model. A
/// request has its method, address, headers and body; a file its path and content.
function ActionFields({ step, steps, inputs, onChange }: KindProps<ActionStep>) {
  return (
    <>
      <Field
        label="It does"
        htmlFor="step-action"
        hint="Sage does this itself: no sol runs, so it uses no tokens."
      >
        <NativeSelect
          id="step-action"
          className="w-full"
          value={step.action}
          onChange={(e) => onChange(withAction(step, e.target.value as ActionStep["action"]))}
        >
          {ACTION_OPTIONS.map(([value, label]) => (
            <NativeSelectOption key={value} value={value}>
              {label}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      {step.action === "http" ? (
        <>
          <div className="grid grid-cols-[7rem_1fr] gap-2">
            <Field label="Method" htmlFor="step-method">
              <NativeSelect
                id="step-method"
                className="w-full"
                value={step.method}
                onChange={(e) => onChange({ ...step, method: e.target.value as HttpMethod })}
              >
                {METHODS.map((m) => (
                  <NativeSelectOption key={m} value={m}>
                    {m.toUpperCase()}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </Field>
            <Field label="Address" htmlFor="step-url">
              <Input
                id="step-url"
                placeholder="https://example.com/send"
                value={step.url}
                onChange={(e) => onChange({ ...step, url: e.target.value })}
              />
            </Field>
          </div>
          <Group
            legend="Headers"
            hint="Sent with the request, such as Authorization. A JSON body needs none: Sage says it is JSON."
          >
            {step.headers.map((header, i) => (
              <div key={i} className="flex gap-2">
                <Input
                  aria-label={`Header ${i + 1} name`}
                  placeholder="Authorization"
                  value={header.name}
                  onChange={(e) =>
                    onChange({
                      ...step,
                      headers: step.headers.map((h, j) =>
                        j === i ? { ...h, name: e.target.value } : h,
                      ),
                    })
                  }
                />
                <Input
                  aria-label={`Header ${i + 1} value`}
                  placeholder="Bearer {{secret.NAME}}"
                  value={header.value}
                  onChange={(e) =>
                    onChange({
                      ...step,
                      headers: step.headers.map((h, j) =>
                        j === i ? { ...h, value: e.target.value } : h,
                      ),
                    })
                  }
                />
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={`Remove header ${i + 1}`}
                  title="Remove"
                  onClick={() =>
                    onChange({ ...step, headers: step.headers.filter((_, j) => j !== i) })
                  }
                >
                  <X aria-hidden />
                </Button>
              </div>
            ))}
            <div>
              <Button
                variant="outline"
                size="sm"
                onClick={() =>
                  onChange({ ...step, headers: [...step.headers, { name: "", value: "" }] })
                }
              >
                <Plus aria-hidden /> Add header
              </Button>
            </div>
          </Group>
          <Field
            label="Body"
            htmlFor="step-body"
            hint="What is sent. In a JSON body, what you put in is written so the JSON stays whole."
          >
            <Textarea
              id="step-body"
              rows={6}
              className="font-mono text-xs"
              value={step.body}
              onChange={(e) => onChange({ ...step, body: e.target.value })}
            />
          </Field>
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <Switch
                id="step-unattended"
                checked={step.unattended}
                onCheckedChange={(unattended) => onChange({ ...step, unattended })}
              />
              <Label htmlFor="step-unattended">Runs without my check</Label>
            </div>
            <p className="text-xs text-muted-foreground">
              Off, the workflow needs a Check step before this one, so you see what goes out
              before it is sent. On, it sends with no one looking.
            </p>
          </div>
          <Marks marks={marksFor(steps, step.id, inputs, true)} />
          <p className="text-xs text-muted-foreground">
            {"{{secret.NAME}}"} is a variable of yours in Library, read when the step runs and
            never kept with the run. A Test run sends nothing.
          </p>
        </>
      ) : (
        <>
          <Field
            label="File"
            htmlFor="step-path"
            hint="Where it is saved, inside the folder the run works in. A file of that name is replaced."
          >
            <Input
              id="step-path"
              placeholder="posts/announcement.md"
              value={step.path}
              onChange={(e) => onChange({ ...step, path: e.target.value })}
            />
          </Field>
          <Field label="Content" htmlFor="step-content">
            <Textarea
              id="step-content"
              rows={6}
              value={step.content}
              onChange={(e) => onChange({ ...step, content: e.target.value })}
            />
          </Field>
          <Marks marks={marksFor(steps, step.id, inputs, false)} />
        </>
      )}
    </>
  );
}

const WAIT_OPTIONS: [WaitStep["wait"], string][] = [
  ["minutes", "For some minutes"],
  ["time", "Until a time of day"],
  ["input", "Until a date and time given at the start"],
];

/// A wait (general workflows spec section 7): how long the workflow waits before its next
/// step. No sol runs and no minutes are counted meanwhile.
function WaitFields({ step, inputs, onChange }: KindProps<WaitStep>) {
  const dated = inputs.filter((f) => f.kind === "date_time");
  return (
    <>
      <Field
        label="It waits"
        htmlFor="step-wait"
        hint="Nothing runs while it waits, and the wait counts no minutes of the budget. Sage must be open for the wait to end; one that ended while it was closed ends when it opens."
      >
        <NativeSelect
          id="step-wait"
          className="w-full"
          value={step.wait}
          onChange={(e) => onChange(withWait(step, e.target.value as WaitStep["wait"], inputs))}
        >
          {WAIT_OPTIONS.map(([value, label]) => (
            <NativeSelectOption key={value} value={value}>
              {label}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </Field>
      {step.wait === "minutes" && (
        <LimitField
          id="step-minutes"
          label="Minutes"
          value={step.minutes}
          onChange={(minutes) => onChange({ ...step, minutes })}
        />
      )}
      {step.wait === "time" && (
        <div className="grid grid-cols-2 gap-2">
          <Field label="Time" htmlFor="step-at" hint="This PC's time.">
            <Input
              id="step-at"
              type="time"
              value={step.at}
              onChange={(e) => onChange({ ...step, at: e.target.value })}
            />
          </Field>
          <Field label="Day" htmlFor="step-weekday">
            <NativeSelect
              id="step-weekday"
              className="w-full"
              value={step.weekday ?? ""}
              onChange={(e) =>
                onChange({
                  ...step,
                  weekday: e.target.value === "" ? null : (e.target.value as Weekday),
                })
              }
            >
              <NativeSelectOption value="">The next time it comes</NativeSelectOption>
              {WEEKDAY_OPTIONS.map(([value, label]) => (
                <NativeSelectOption key={value} value={value}>
                  {label}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </div>
      )}
      {step.wait === "input" &&
        (dated.length === 0 && step.input === "" ? (
          <p role="note" className="text-sm text-muted-foreground">
            Add an input of the kind "A date and time" to the workflow first: the run waits
            until what it is given there.
          </p>
        ) : (
          <Field label="Input" htmlFor="step-input">
            <NativeSelect
              id="step-input"
              className="w-full"
              value={step.input}
              onChange={(e) => onChange({ ...step, input: e.target.value })}
            >
              {!dated.some((f) => f.name === step.input) && (
                <NativeSelectOption value={step.input}>
                  {step.input === "" ? "Pick an input" : `${step.input} (not a date and time)`}
                </NativeSelectOption>
              )}
              {dated.map((f) => (
                <NativeSelectOption key={f.name} value={f.name}>
                  {f.label.trim() || f.name}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        ))}
    </>
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
            hint="The sol of the team that deploys what the check approved."
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
