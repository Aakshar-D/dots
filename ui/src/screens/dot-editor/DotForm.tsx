import { open } from "@tauri-apps/plugin-dialog";
import type { ReactNode } from "react";

import {
  EngineKind,
  WorkspaceMode,
  type Dot,
  type DotSpec,
  type Schedule,
  type ScheduleRule,
} from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { cronSchedule, type SpecErrors } from "@/lib/dot-rules";
import { localDateTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds: the spec, and the MCP servers as the text being typed.
export type Draft = { spec: DotSpec; mcpText: string };

export type SchedulePreview = { times: string[]; pending: boolean };

const localDate = (dateTime: string) => dateTime.replace("T", " ");

function describeRule(rule: ScheduleRule): string {
  switch (rule.kind) {
    case "once":
      return `Once at ${localDate(rule.at)}`;
    case "every":
      return `Every ${rule.minutes} minutes`;
    case "daily":
      return `Daily at ${rule.time}`;
    case "weekly":
      return `Weekly on ${rule.days.join(", ")} at ${rule.time}`;
    case "cron":
      return `Cron ${rule.expr}`;
  }
}

/// A schedule in words, for the kinds this form cannot edit yet.
function describeSchedule({ rule, until }: Schedule): string {
  const what = describeRule(rule);
  if (until === null) return what;
  return "count" in until
    ? `${what}, at most ${until.count} runs`
    : `${what}, until ${localDate(until.date)}`;
}

function Section(props: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">{props.title}</legend>
      {props.children}
    </fieldset>
  );
}

function NumberField(props: {
  id: string;
  label: string;
  value: number;
  onChange: (value: number) => void;
  error?: string;
  hint?: string;
}) {
  return (
    <Field label={props.label} htmlFor={props.id} error={props.error} hint={props.hint}>
      <Input
        id={props.id}
        type="number"
        className="w-40"
        value={Number.isNaN(props.value) ? "" : props.value}
        onChange={(e) =>
          props.onChange(e.target.value === "" ? Number.NaN : Number(e.target.value))
        }
      />
    </Field>
  );
}

/// Every field of a dot. `existing` is set once the dot is stored: only then does it have a
/// webhook.
export function DotForm(props: {
  draft: Draft;
  onChange: (draft: Draft) => void;
  errors: SpecErrors;
  existing: Dot | null;
  endpoints: string[];
  schedule: SchedulePreview;
}) {
  const { draft, errors } = props;
  const { spec } = draft;
  const set = <K extends keyof DotSpec>(key: K, value: DotSpec[K]) =>
    props.onChange({ ...draft, spec: { ...spec, [key]: value } });

  function browse() {
    open({ directory: true, title: "Choose the folder the dot works in" }).then((picked) => {
      if (typeof picked === "string") set("workdir", picked);
    }, notifyError);
  }

  return (
    <div className="flex max-w-3xl flex-col gap-4">
      <Section title="Basics">
        <Field label="Name" htmlFor="dot-name" error={errors.name}>
          <Input id="dot-name" value={spec.name} onChange={(e) => set("name", e.target.value)} />
        </Field>
        <Field
          label="Instructions"
          htmlFor="dot-instructions"
          error={errors.instructions}
          hint="What the dot should do on each run. Markdown."
        >
          <Textarea
            id="dot-instructions"
            rows={8}
            value={spec.instructions}
            onChange={(e) => set("instructions", e.target.value)}
          />
        </Field>
        <div className="flex items-center gap-2">
          <Switch
            id="dot-enabled"
            checked={spec.enabled}
            onCheckedChange={(v) => set("enabled", v)}
          />
          <Label htmlFor="dot-enabled">Enabled</Label>
        </div>
      </Section>

      <Section title="Engine">
        <Field label="Engine" htmlFor="dot-engine">
          <NativeSelect
            id="dot-engine"
            value={spec.engine}
            onChange={(e) => set("engine", EngineKind.parse(e.target.value))}
          >
            <option value="claude">Claude Code CLI (your subscription)</option>
            <option value="local">Local model (OpenAI-compatible endpoint)</option>
          </NativeSelect>
        </Field>
        <Field label="Model" htmlFor="dot-model" error={errors.model}>
          <Input
            id="dot-model"
            list={spec.engine === "claude" ? "claude-models" : undefined}
            value={spec.model}
            onChange={(e) => set("model", e.target.value)}
          />
          <datalist id="claude-models">
            <option value="sonnet" />
            <option value="opus" />
            <option value="haiku" />
          </datalist>
        </Field>
        {spec.engine === "local" && (
          <Field
            label="Endpoint"
            htmlFor="dot-endpoint"
            error={errors.endpoint_url}
            hint="For example http://127.0.0.1:1234 (LM Studio). Endpoints from Settings are suggested."
          >
            <Input
              id="dot-endpoint"
              list="local-endpoints"
              value={spec.endpoint_url ?? ""}
              onChange={(e) => set("endpoint_url", e.target.value)}
            />
            <datalist id="local-endpoints">
              {props.endpoints.map((url) => (
                <option key={url} value={url} />
              ))}
            </datalist>
          </Field>
        )}
      </Section>

      <Section title="Workspace">
        <Field label="Folder" htmlFor="dot-workdir" error={errors.workdir}>
          <div className="flex gap-2">
            <Input
              id="dot-workdir"
              className="font-mono"
              value={spec.workdir}
              onChange={(e) => set("workdir", e.target.value)}
            />
            <Button type="button" variant="outline" onClick={browse}>
              Browse…
            </Button>
          </div>
        </Field>
        <Field
          label="Mode"
          htmlFor="dot-mode"
          hint={
            spec.workspace_mode === "worktree"
              ? "Each run works on its own branch in a separate git worktree. You review the diff, then keep or discard it. The folder must be a git repository."
              : "Runs work directly in the folder. There is nothing to review or discard."
          }
        >
          <NativeSelect
            id="dot-mode"
            value={spec.workspace_mode}
            onChange={(e) => set("workspace_mode", WorkspaceMode.parse(e.target.value))}
          >
            <option value="worktree">Git worktree per run</option>
            <option value="folder">The folder itself</option>
          </NativeSelect>
        </Field>
      </Section>

      <Section title="Schedule">
        {spec.schedule && spec.schedule.rule.kind !== "cron" ? (
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center gap-3">
              <span className="text-sm">{describeSchedule(spec.schedule)}</span>
              <Button size="sm" variant="outline" onClick={() => set("schedule", null)}>
                Remove schedule
              </Button>
            </div>
            {errors.schedule ? (
              <p className="text-xs text-destructive">{errors.schedule}</p>
            ) : (
              <p className="text-xs text-muted-foreground">
                This editor cannot change this kind of schedule yet. Saving keeps it as it is.
              </p>
            )}
          </div>
        ) : (
          <Field
            label="Cron expression"
            htmlFor="dot-schedule"
            error={errors.schedule}
            hint="Six fields: second minute hour day-of-month month day-of-week. Empty: no schedule."
          >
            <Input
              id="dot-schedule"
              className="font-mono"
              placeholder="0 0 2 * * *"
              value={spec.schedule?.rule.kind === "cron" ? spec.schedule.rule.expr : ""}
              onChange={(e) => set("schedule", cronSchedule(e.target.value))}
            />
          </Field>
        )}
        {props.schedule.times.length > 0 && (
          <div className="text-xs text-muted-foreground">
            <span>Next runs:</span>
            <ul aria-label="Next runs" className="mt-1 flex flex-col gap-0.5">
              {props.schedule.times.map((t) => (
                <li key={t}>{localDateTime(t)}</li>
              ))}
            </ul>
          </div>
        )}
      </Section>

      {props.existing && (
        <Section title="Webhook">
          <WebhookPanel dot={props.existing} />
        </Section>
      )}

      <Section title="Permissions">
        <PolicyEditor
          policy={spec.policy}
          onChange={(policy) => set("policy", policy)}
          error={errors.policy}
        />
      </Section>

      <Section title="Limits">
        <NumberField
          id="dot-max-turns"
          label="Max turns"
          value={spec.max_turns}
          onChange={(v) => set("max_turns", v)}
          error={errors.max_turns}
        />
        <NumberField
          id="dot-timeout"
          label="Timeout (seconds)"
          value={spec.timeout_secs}
          onChange={(v) => set("timeout_secs", v)}
          error={errors.timeout_secs}
        />
        <NumberField
          id="dot-approval-wait"
          label="Approval wait (seconds)"
          value={spec.approval_wait_secs}
          onChange={(v) => set("approval_wait_secs", v)}
          error={errors.approval_wait_secs}
          hint="How long a run waits for your answer before it parks and continues later."
        />
      </Section>

      {spec.engine === "claude" && (
        <Section title="Claude Code">
          <Field
            label="MCP servers"
            htmlFor="dot-mcp"
            error={errors.mcp_servers}
            hint='A JSON object, as in a Claude Code "mcpServers" block. Empty: none.'
          >
            <Textarea
              id="dot-mcp"
              rows={5}
              className="font-mono"
              value={draft.mcpText}
              onChange={(e) => props.onChange({ ...draft, mcpText: e.target.value })}
            />
          </Field>
          <div className="flex items-center gap-2">
            <Switch
              id="dot-user-settings"
              checked={spec.use_user_settings}
              onCheckedChange={(v) => set("use_user_settings", v)}
            />
            <Label htmlFor="dot-user-settings">Load my Claude Code user settings</Label>
          </div>
        </Section>
      )}
    </div>
  );
}
