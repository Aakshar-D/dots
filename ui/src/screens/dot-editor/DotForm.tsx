import { open } from "@tauri-apps/plugin-dialog";
import type { ReactNode } from "react";

import { EngineKind, WorkspaceMode, type Dot, type DotSpec } from "@/api/types";
import { Field } from "@/components/Field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SpecErrors } from "@/lib/dot-rules";
import { notifyError } from "@/lib/notify";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds. The Library items the dot uses (`spec.uses`) are kept as they are.
export type Draft = { spec: DotSpec };

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
        <div className="flex items-center gap-2">
          <Switch
            id="dot-global-instructions"
            checked={spec.use_global_instructions}
            onCheckedChange={(v) => set("use_global_instructions", v)}
          />
          <Label htmlFor="dot-global-instructions">Start with the global instructions</Label>
        </div>
        <p className="-mt-2 text-xs text-muted-foreground">
          The global instructions are set in Settings. A blank text adds nothing.
        </p>
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
        <ScheduleEditor
          schedule={spec.schedule}
          stored={props.existing?.schedule ?? null}
          onChange={(schedule) => set("schedule", schedule)}
          error={errors.schedule}
          preview={props.schedule}
        />
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

      <Section title="Tools and skills">
        <ToolsAndSkills
          uses={spec.uses}
          engine={spec.engine}
          onChange={(uses) => set("uses", uses)}
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
