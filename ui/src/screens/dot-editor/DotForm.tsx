import { useQuery } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useState, type ReactNode } from "react";

import { commands } from "@/api/commands";
import { EngineKind, WorkspaceMode, type Dot, type DotSpec } from "@/api/types";
import { Field } from "@/components/Field";
import { ContextRulesFields } from "@/components/ContextMeter";
import { ModelPicker, useModels } from "@/components/ModelPicker";
import { CapInput, SpendLine, useSpendToday } from "@/components/SpendCaps";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SpecErrors } from "@/lib/dot-rules";
import { notifyError } from "@/lib/notify";
import { tokenCapError, usdCapError } from "@/lib/spend";
import { keys } from "@/lib/query";
import { BrowserSection } from "@/screens/dot-editor/BrowserSection";
import { PERSONAL } from "@/lib/scope";
import { OrgLine, OrgPicker, WorkspaceMove, WorkspacePicker } from "@/screens/dot-editor/DotOrg";
import { PolicyEditor } from "@/screens/dot-editor/PolicyEditor";
import { ScheduleEditor, type SchedulePreview } from "@/screens/dot-editor/ScheduleEditor";
import { ToolsAndSkills } from "@/screens/dot-editor/ToolsAndSkills";
import { Variables } from "@/screens/dot-editor/Variables";
import { WebhookPanel } from "@/screens/dot-editor/WebhookPanel";

/// What the editor holds. The Library items the dot uses (`spec.uses`) are kept as they are.
/// `org` is the org a new dot is made in (null for Personal), and `workspace` its workspace
/// there (null for the default one); a stored dot's are its own.
export type Draft = { spec: DotSpec; org: string | null; workspace: string | null };

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

type QuietMode = "default" | "off" | "minutes";
const quietMode = (secs: number | null): QuietMode =>
  secs === null ? "default" : secs === 0 ? "off" : "minutes";

/// The dot's quiet limit: the app default, off, or a number of minutes. The spec holds seconds.
function QuietLimit(props: {
  spec: DotSpec;
  onChange: (secs: number | null) => void;
  error?: string;
}) {
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data;
  const { spec } = props;
  const secs = spec.quiet_limit_secs;
  // The select's mode is held here: it is derived from the stored seconds only at first, so
  // typing 0 minutes does not flip the select to Off.
  const [mode, setMode] = useState<QuietMode>(() => quietMode(secs));
  const [minutes, setMinutes] = useState(() => (secs ? String(secs / 60) : "10"));
  const defaultLabel = settings
    ? `App default (${settings.quiet_limit_secs / 60} min)`
    : "App default";
  const longer = secs !== null && secs > spec.timeout_secs;
  return (
    <Field
      label="Quiet limit"
      htmlFor="dot-quiet-mode"
      error={props.error}
      hint={
        <>
          If this dot shows no output for this long, Dots asks you; at twice this long it stops
          the run.
          {longer && " Longer than the time limit; the time limit will end the run first."}
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <NativeSelect
          id="dot-quiet-mode"
          className="w-52"
          value={mode}
          onChange={(e) => {
            const next = e.target.value as QuietMode;
            setMode(next);
            if (next === "minutes") {
              setMinutes("10");
              props.onChange(600);
            } else {
              props.onChange(next === "default" ? null : 0);
            }
          }}
        >
          <option value="default">{defaultLabel}</option>
          <option value="off">Off</option>
          <option value="minutes">Minutes</option>
        </NativeSelect>
        {mode === "minutes" && (
          <Input
            id="dot-quiet-minutes"
            aria-label="Quiet limit (minutes)"
            type="number"
            className="w-28"
            value={minutes}
            onChange={(e) => {
              setMinutes(e.target.value);
              // Empty or 0 minutes is not a limit and not Off: it is invalid until fixed.
              const n = e.target.value === "" ? Number.NaN : Number(e.target.value);
              props.onChange(n === 0 ? Number.NaN : n * 60);
            }}
          />
        )}
      </div>
    </Field>
  );
}

/// The Claude CLI's model aliases (`dots_core::models::CLAUDE_MODELS`), and Auto, which only a
/// Claude dot has.
const CLAUDE_ALIASES = ["auto", "opus", "sonnet", "haiku"];
const isClaudeModel = (model: string) => {
  const m = model.trim().toLowerCase();
  return CLAUDE_ALIASES.includes(m) || m.startsWith("claude-");
};

/// The model a dot keeps when its engine changes from `from` to `to` (model switcher spec
/// section 2.3): a Claude dot given a model no Claude engine runs starts with the default
/// model; a local dot drops a Claude model, so the person picks one the endpoint lists.
export function modelFor(
  to: EngineKind,
  from: EngineKind,
  model: string,
  defaultModel: string | undefined,
): string {
  if (to === from) return model;
  if (to === "claude") return isClaudeModel(model) ? model : (defaultModel ?? "sonnet");
  return isClaudeModel(model) ? "" : model;
}

/// The dot's daily caps (spend caps design) and what it spent today. Empty is no cap.
function SpendingCaps(props: {
  dotId: string | null;
  spec: DotSpec;
  onChange: (caps: DotSpec["spend_cap"]) => void;
}) {
  const caps = props.spec.spend_cap;
  const spend = useSpendToday().data;
  const today = spend?.dots.find((d) => d.dot_id === props.dotId)?.today ?? {
    cost_usd: 0,
    tokens: 0,
  };
  return (
    <>
      <p className="text-xs text-muted-foreground">
        Once the dot reaches a cap, its new runs wait until local midnight or until you raise the
        cap, and a run that goes over is stopped. Your own chats can run once anyway. Costs are
        estimates: what the runs would cost on the API. Settings can cap all dots together.
      </p>
      {props.dotId !== null && spend && (
        <SpendLine
          label="Today"
          spend={today}
          usdCap={caps.daily_usd}
          tokenCap={caps.daily_tokens}
          showTokens
        />
      )}
      <div className="flex flex-wrap gap-4">
        <CapInput
          id="dot-cap-usd"
          label="Daily cost cap (USD)"
          value={caps.daily_usd}
          onChange={(daily_usd) => props.onChange({ ...caps, daily_usd })}
          error={usdCapError(caps.daily_usd)}
        />
        <CapInput
          id="dot-cap-tokens"
          label="Daily token cap"
          value={caps.daily_tokens}
          onChange={(daily_tokens) => props.onChange({ ...caps, daily_tokens })}
          error={tokenCapError(caps.daily_tokens)}
        />
      </div>
    </>
  );
}

/// Every field of a dot. `existing` is set once the dot is stored: only then does it have a
/// webhook, and its org is fixed. `onEnabled`, when given, switches the dot on or off at once
/// instead of with the next save: for a dot that can't be saved.
export function DotForm(props: {
  draft: Draft;
  onChange: (draft: Draft) => void;
  errors: SpecErrors;
  existing: Dot | null;
  endpoints: string[];
  schedule: SchedulePreview;
  onEnabled?: (enabled: boolean) => void;
}) {
  const { draft, errors } = props;
  const { spec } = draft;
  const orgId = props.existing ? props.existing.org_id : draft.org;
  const set = <K extends keyof DotSpec>(key: K, value: DotSpec[K]) =>
    props.onChange({ ...draft, spec: { ...spec, [key]: value } });
  const models = useModels(spec.engine, spec.endpoint_url);
  const settings = useQuery({ queryKey: keys.settings, queryFn: commands.getSettings }).data;

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
        {props.existing ? (
          <>
            {props.existing.org_id !== null && <OrgLine orgId={props.existing.org_id} />}
            <WorkspaceMove dot={props.existing} />
          </>
        ) : (
          <>
            <OrgPicker
              value={draft.org}
              onChange={(org) => props.onChange({ ...draft, org, workspace: null })}
            />
            <WorkspacePicker
              account={draft.org ?? PERSONAL}
              value={draft.workspace}
              onChange={(workspace) => props.onChange({ ...draft, workspace })}
            />
          </>
        )}
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
            onCheckedChange={(v) => (props.onEnabled ? props.onEnabled(v) : set("enabled", v))}
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
            onChange={(e) => {
              const engine = EngineKind.parse(e.target.value);
              props.onChange({
                ...draft,
                spec: {
                  ...spec,
                  engine,
                  model: modelFor(engine, spec.engine, spec.model, settings?.default_model),
                },
              });
            }}
          >
            <option value="claude">Claude Code CLI (your subscription)</option>
            <option value="local">Local model (OpenAI-compatible endpoint)</option>
          </NativeSelect>
        </Field>
        <Field label="Model" htmlFor="dot-model" error={errors.model}>
          <ModelPicker
            id="dot-model"
            value={spec.model}
            onChange={(model) => set("model", model)}
            options={models.options}
            note={models.note}
          />
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
        {spec.engine === "claude" && (
          <>
            <div className="flex items-center gap-2">
              <Switch
                id="dot-check-results"
                checked={spec.check_results}
                onCheckedChange={(v) => set("check_results", v)}
              />
              <Label htmlFor="dot-check-results">Check its results</Label>
            </div>
            <p className="-mt-2 text-xs text-muted-foreground">
              A quick automatic second look at each answer before it reaches you, while Check
              every result is on in Settings.
            </p>
          </>
        )}
      </Section>

      <Section title="Workspace">
        <Field
          label="Folder"
          htmlFor="dot-workdir"
          error={errors.workdir}
          hint={props.existing ? undefined : "Leave blank and Dots makes a new folder for this dot."}
        >
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
        <div className="flex items-center gap-2">
          <Switch
            id="dot-use-graph"
            checked={spec.use_graph}
            onCheckedChange={(v) => set("use_graph", v)}
          />
          <Label htmlFor="dot-use-graph">Use project graph</Label>
        </div>
        <p className="-mt-2 text-xs text-muted-foreground">
          Dots keeps a code graph of the folder. Runs get a short map of the project and a graph
          tool, so they read and search less.
        </p>
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
          engine={spec.engine}
          onChange={(policy) => set("policy", policy)}
          error={errors.policy}
        />
      </Section>

      <Section title="Tools and skills">
        <ToolsAndSkills
          uses={spec.uses}
          engine={spec.engine}
          orgId={orgId}
          onChange={(uses) => set("uses", uses)}
        />
      </Section>

      <Section title="Variables">
        <Variables
          uses={spec.uses}
          variables={spec.variables}
          before={props.existing?.uses ?? null}
          orgId={orgId}
          dotId={props.existing?.id ?? null}
          onChange={(variables) => set("variables", variables)}
        />
      </Section>

      <Section title="Browser">
        <BrowserSection
          spec={spec}
          existing={props.existing}
          onChange={(next) => props.onChange({ ...draft, spec: next })}
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
        <QuietLimit
          spec={spec}
          onChange={(secs) => set("quiet_limit_secs", secs)}
          error={errors.quiet_limit_secs}
        />
      </Section>

      <Section title="Spending caps">
        <SpendingCaps
          dotId={props.existing?.id ?? null}
          spec={spec}
          onChange={(spend_cap) => set("spend_cap", spend_cap)}
        />
      </Section>

      {spec.engine === "claude" && (
        <Section title="Context">
          <div className="flex items-center gap-2">
            <Switch
              id="dot-context-own"
              checked={spec.context_rules !== null}
              onCheckedChange={(own) =>
                set(
                  "context_rules",
                  own ? (settings?.context_rules ?? { enabled: true, compact_at: 60, clear_at: 80 }) : null,
                )
              }
            />
            <Label htmlFor="dot-context-own">Use its own context rules</Label>
          </div>
          {spec.context_rules === null ? (
            <p className="text-xs text-muted-foreground">
              It follows Settings › Runs &amp; server › Context
              {settings &&
                (settings.context_rules.enabled
                  ? `: compact at ${settings.context_rules.compact_at}%, start fresh at ${settings.context_rules.clear_at}%.`
                  : ": off.")}
            </p>
          ) : (
            <ContextRulesFields
              id="dot-context"
              value={spec.context_rules}
              onChange={(rules) => set("context_rules", rules)}
              error={errors.context_rules}
            />
          )}
        </Section>
      )}

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
