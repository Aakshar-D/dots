import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { Schedule, Team, Trigger, Workflow } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Textarea } from "@/components/ui/textarea";
import { scheduleError, scheduleOf, withKind } from "@/lib/dot-rules";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { useDebounced } from "@/lib/use-debounced";
import { ScheduleEditor } from "@/screens/dot-editor/ScheduleEditor";
import { CopyButton } from "@/screens/dot-editor/WebhookPanel";

import { TRIGGER_LABEL } from "../format";
import { MAX_TASK, toggled, withTriggerKind } from "./draft";
import { ChoiceList } from "./fields";
import type { NameOf } from "./labels";

const KINDS: Trigger["kind"][] = ["manual", "chat", "schedule", "webhook"];

/// What each kind of trigger does (section 5.1), under the kind's field.
const KIND_HINT: Record<Trigger["kind"], string> = {
  manual: "Run now on the team page, or Test run here: you type the task and pick the project.",
  chat: "A member starts it with start_workflow while you chat with it.",
  schedule:
    "It runs at each time of the schedule. A time that comes while a run is still in flight is skipped.",
  webhook: "Another program starts it with a POST request to its URL.",
};

/// The side sheet of the trigger's settings (section 10.2), open while `open`: its kind and the
/// kind's fields. It edits the draft in place. `workflow` is the saved workflow: its webhook
/// URL and token are the saved workflow's, and they take calls once the draft is saved.
export function TriggerSheet(props: {
  open: boolean;
  trigger: Trigger;
  enabled: boolean;
  workflow: Workflow;
  team: Team;
  nameOf: NameOf;
  onChange: (trigger: Trigger) => void;
  onClose: () => void;
}) {
  const { trigger, team, onChange } = props;
  return (
    <Sheet open={props.open} onOpenChange={(open) => !open && props.onClose()}>
      {props.open && (
        <SheetContent>
          <SheetHeader>
            <SheetTitle>Trigger settings</SheetTitle>
            <SheetDescription>
              Changes go into the draft at once. Save keeps them.
            </SheetDescription>
          </SheetHeader>
          <Field label="What starts it" htmlFor="trigger-kind" hint={KIND_HINT[trigger.kind]}>
            <NativeSelect
              id="trigger-kind"
              className="w-full"
              value={trigger.kind}
              onChange={(e) => {
                const kind = KINDS.find((k) => k === e.target.value);
                // A new schedule starts daily at 09:00, as the dot editor's does, with no
                // catch-up: a workflow's schedule never catches up.
                const daily = { ...(withKind(null, "daily", null) as Schedule), catch_up: false };
                if (kind) onChange(withTriggerKind(trigger, kind, team, daily));
              }}
            >
              {KINDS.map((kind) => (
                <NativeSelectOption key={kind} value={kind}>
                  {TRIGGER_LABEL[kind]}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
          {!props.enabled && (
            <p role="note" className="text-xs text-muted-foreground">
              The workflow is off: this trigger starts nothing until you turn it on and save. A
              Test run works while it is off.
            </p>
          )}
          {trigger.kind === "chat" && (
            <ChoiceList
              legend="Started by"
              hint="The members who may start it from a chat with you."
              options={team.members.map((m) => ({ value: m.dot_id, label: m.dot_name }))}
              chosen={trigger.starters}
              stale={props.nameOf}
              empty="The team has no member yet."
              onChange={(id, on) =>
                onChange({ ...trigger, starters: toggled(trigger.starters, id, on) })
              }
            />
          )}
          {trigger.kind === "schedule" && (
            <ScheduleFields
              trigger={trigger}
              stored={props.workflow.definition.trigger}
              team={team}
              onChange={onChange}
            />
          )}
          {trigger.kind === "webhook" && (
            <>
              <WebhookFields
                workflow={props.workflow}
                saved={props.workflow.definition.trigger.kind === "webhook"}
              />
              <Field
                label="Task when the call has none"
                htmlFor="trigger-task"
                hint="Used when the body has no task. Leave it empty to make every call say one."
              >
                <Textarea
                  id="trigger-task"
                  maxLength={MAX_TASK}
                  value={trigger.task ?? ""}
                  onChange={(e) => onChange({ ...trigger, task: e.target.value || null })}
                />
              </Field>
              <ProjectSelect
                label="Project when the call has none"
                team={team}
                value={trigger.project ?? ""}
                none="None: the call names it"
                onChange={(project) => onChange({ ...trigger, project: project || null })}
              />
            </>
          )}
        </SheetContent>
      )}
    </Sheet>
  );
}

/// A project of the team, picked and kept by name (a trigger names its project). `none`, when
/// given, is the choice of no project; else none picked yet asks for one. A name the team no
/// longer has stays shown until another is picked.
function ProjectSelect(props: {
  label: string;
  team: Team;
  value: string;
  none?: string;
  onChange: (name: string) => void;
}) {
  const { value } = props;
  const known = props.team.projects.some((p) => p.name === value);
  return (
    <Field label={props.label} htmlFor="trigger-project">
      <NativeSelect
        id="trigger-project"
        className="w-full"
        value={value}
        onChange={(e) => props.onChange(e.target.value)}
      >
        {props.none !== undefined ? (
          <NativeSelectOption value="">{props.none}</NativeSelectOption>
        ) : (
          value === "" && (
            <NativeSelectOption value="" disabled>
              Pick a project
            </NativeSelectOption>
          )
        )}
        {!known && value !== "" && (
          <NativeSelectOption value={value}>{value} (not a project of the team)</NativeSelectOption>
        )}
        {props.team.projects.map((p) => (
          <NativeSelectOption key={p.id} value={p.name}>
            {p.name}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Field>
  );
}

type ScheduleTrigger = Extract<Trigger, { kind: "schedule" }>;

/// A schedule trigger: the dot editor's schedule fields (always a schedule, never a catch-up),
/// with the core's next runs, then the fixed task and project of each run.
function ScheduleFields(props: {
  trigger: ScheduleTrigger;
  stored: Trigger;
  team: Team;
  onChange: (trigger: Trigger) => void;
}) {
  const { trigger, onChange } = props;
  // As the dot editor does: the core previews the schedule once it settles; a field the core
  // could not even be sent (an emptied number) is held back with the UI's own message.
  const schedule = scheduleOf(useDebounced(trigger.schedule, 300));
  const typed = schedule ? scheduleError(schedule) : undefined;
  const preview = useQuery({
    queryKey: keys.schedulePreview(schedule),
    queryFn: () => (schedule ? commands.previewSchedule(schedule, 3) : []),
    enabled: typed === undefined,
  });
  const error =
    scheduleError(trigger.schedule) ??
    (typed === undefined && preview.isError ? errorMessage(preview.error) : undefined);
  return (
    <>
      <ScheduleEditor
        schedule={trigger.schedule}
        stored={props.stored.kind === "schedule" ? props.stored.schedule : null}
        onChange={(next) => next && onChange({ ...trigger, schedule: next })}
        error={error}
        preview={{
          times: typed === undefined && preview.data ? preview.data : [],
          pending: preview.isFetching,
        }}
        required
        catchUp={false}
      />
      <Field label="Task" htmlFor="trigger-task" hint="What each run of the schedule works on.">
        <Textarea
          id="trigger-task"
          maxLength={MAX_TASK}
          value={trigger.task}
          onChange={(e) => onChange({ ...trigger, task: e.target.value })}
        />
      </Field>
      <ProjectSelect
        label="Project"
        team={props.team}
        value={trigger.project}
        onChange={(project) => onChange({ ...trigger, project })}
      />
    </>
  );
}

/// How another program starts the saved workflow: its URL and bearer token, as a dot's
/// `WebhookPanel` shows them. Dots keeps only the token's hash, so a token is shown once, when
/// it is made. `saved`: the saved workflow has a webhook trigger, so calls reach it.
function WebhookFields({ workflow, saved }: { workflow: Workflow; saved: boolean }) {
  const qc = useQueryClient();
  const [shown, setShown] = useState<string | null>(null);
  const url = useQuery({
    queryKey: keys.workflowWebhookUrl(workflow.id),
    queryFn: () => commands.workflowWebhookUrl(workflow.id),
  });
  const regenerate = useMutation({
    mutationFn: () => commands.regenerateWorkflowToken(workflow.id),
    onSuccess: (token) => {
      setShown(token);
      qc.setQueryData<Workflow>(keys.workflow(workflow.id), (w) =>
        w ? { ...w, has_webhook_token: true } : w,
      );
      invalidate(qc, keys.workflows(workflow.team_id));
      toast.success(
        workflow.has_webhook_token ? "New token. The old one no longer works." : "Token made.",
      );
    },
    onError: notifyError,
  });
  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-muted-foreground">
        Send a POST request to the URL with the header{" "}
        <code>Authorization: Bearer &lt;token&gt;</code>. The body's <code>task</code> and{" "}
        <code>project</code> (by name) are used; the fallbacks below fill in what it leaves out.
      </p>
      {!saved && (
        <p role="note" className="text-xs text-needs">
          Calls are refused until the workflow is saved with this trigger.
        </p>
      )}
      <Field
        label="Webhook URL"
        htmlFor="workflow-webhook-url"
        error={url.isError ? errorMessage(url.error) : undefined}
      >
        <div className="flex gap-2">
          <Input
            id="workflow-webhook-url"
            readOnly
            className="font-mono"
            value={url.data ?? ""}
          />
          {url.data && <CopyButton what="webhook URL" text={url.data} />}
        </div>
      </Field>
      {shown !== null ? (
        <Field
          label="Token"
          htmlFor="workflow-webhook-token"
          hint="Copy it now: you won't see this again."
        >
          <div className="flex gap-2">
            <Input id="workflow-webhook-token" readOnly className="font-mono" value={shown} />
            <CopyButton what="token" text={shown} />
          </div>
        </Field>
      ) : (
        <p className="text-sm text-muted-foreground">
          {workflow.has_webhook_token
            ? "This workflow has a token. It was shown once, when it was made."
            : "This workflow has no token yet, so its webhook refuses every call."}
        </p>
      )}
      <div>
        {workflow.has_webhook_token ? (
          <ConfirmButton
            title="Make a new token?"
            description="Anything that still sends the old token will be refused."
            confirmLabel="Make a new token"
            onConfirm={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Regenerate
          </ConfirmButton>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => regenerate.mutate()}
            disabled={regenerate.isPending}
          >
            Make a token
          </Button>
        )}
      </div>
    </div>
  );
}
