import { useQuery } from "@tanstack/react-query";
import { useId, useState } from "react";

import { commands } from "@/api/commands";
import type { GitAction, Guardrails, GuardrailsView, OrgSummary, Preset } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { capitalized, lines, MAX_PRESETS, PRESET_NAMES } from "@/lib/guardrails";
import { keys } from "@/lib/query";

import { BUSY, DRAFT_WAITING, DraftBanner, rosterBlockers, useRosterEdits } from "./AccountMembers";

/// What turning guardrails off does, asked before it is signed.
export const OFF_TEXT =
  "Its sols go back to their own policies: nothing is protected, and a Member's sol may use any preset.";

/// The Guardrails tab of an account (guardrails spec section 7): one workspace at a time, its
/// protected paths, git actions and commands, and the highest preset a Member's dot may use.
/// An Admin or Owner whose Dots ID signs changes them as a signed member-list edit; everyone
/// else sees them as they are, with the core's reason.
export function AccountGuardrails({ org }: { org: OrgSummary }) {
  const spaces = useQuery({
    queryKey: keys.workspaces(org.id),
    queryFn: () => commands.workspaceList(org.id),
  });
  const [picked, setPicked] = useState("default");
  if (spaces.isError) return <ErrorNote error={spaces.error} />;
  const items = spaces.data;
  if (!items) return <p className="text-xs text-muted-foreground">Loading the workspaces…</p>;
  // A workspace deleted meanwhile falls back to Default.
  const workspaceId = items.some((w) => w.id === picked) ? picked : "default";
  return (
    <div className="flex flex-col gap-3 pt-1">
      {items.length > 1 && (
        <Field label="Workspace" htmlFor={`guardrails-workspace-${org.id}`}>
          <NativeSelect
            id={`guardrails-workspace-${org.id}`}
            size="sm"
            value={workspaceId}
            onChange={(e) => setPicked(e.target.value)}
          >
            {items.map((w) => (
              <NativeSelectOption key={w.id} value={w.id}>
                {w.name}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </Field>
      )}
      <WorkspaceGuardrails key={workspaceId} org={org} workspaceId={workspaceId} />
    </div>
  );
}

function WorkspaceGuardrails({ org, workspaceId }: { org: OrgSummary; workspaceId: string }) {
  const { list, notice, act } = useRosterEdits(org);
  const guard = useQuery({
    queryKey: keys.orgGuardrails(org.id, workspaceId),
    queryFn: () => commands.workspaceGuardrails(org.id, workspaceId),
  });
  if (guard.isError) return <ErrorNote error={guard.error} />;
  if (list.isError) return <ErrorNote error={list.error} />;
  const view = guard.data;
  const members = list.data;
  if (!view || !members) {
    return <p className="text-xs text-muted-foreground">Loading the guardrails…</p>;
  }
  const busy = act.isPending;
  const blockers = rosterBlockers(org, members);
  // The core's reasons first: the role, then the Dots ID. A draft waiting comes after them.
  const readOnly = view.edit_refusal ?? view.sign_blocker ?? undefined;
  const lock =
    readOnly ??
    (members.draft ? `${org.name}: ${DRAFT_WAITING}` : undefined) ??
    (busy ? BUSY : undefined);
  const save = (guardrails: Guardrails | null) =>
    act.mutate(() =>
      commands.workspaceGuardrailsSet(org.id, view.seq, view.workspace_id, guardrails),
    );
  return (
    <section aria-label={`Guardrails of ${view.workspace_name}`} className="flex flex-col gap-3">
      {members.draft && (
        <DraftBanner
          view={members}
          blocker={blockers.blocker}
          discardBlocker={blockers.manageBlocker}
          busy={busy}
          retry={() => act.mutate(() => commands.rosterDraftRetry(org.id))}
          discard={() => act.mutate(() => commands.rosterDraftDiscard(org.id))}
        />
      )}
      {notice && (
        <p role="alert" className="text-sm text-destructive">
          {notice}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        {appliesText(view)} Member list #{view.seq}.
      </p>
      {readOnly && <p className="text-xs text-needs">{readOnly}</p>}
      {readOnly ? (
        <GuardrailsSummary view={view} />
      ) : (
        // A new list (a save, or another Admin's change met as a conflict) starts the form
        // again from what is on it.
        <GuardrailsForm key={view.seq} view={view} lock={lock} onSave={save} />
      )}
    </section>
  );
}

/// How the guardrails treat this PC's dots in the workspace, in a sentence.
function appliesText(view: GuardrailsView): string {
  if (view.guardrails === null) return `${view.workspace_name} has no guardrails.`;
  switch (view.applies_as) {
    case "member":
      return "Your sols here are guarded as a Member's: protected calls are refused.";
    case "admin":
      return "Your sols here always ask before a protected call.";
    default:
      return "They don't apply to sols on this PC.";
  }
}

/// The guardrails as they are, for a PC that may not change them.
function GuardrailsSummary({ view }: { view: GuardrailsView }) {
  const g = view.guardrails;
  if (g === null) return null;
  // Capitalised as the form's checkboxes are.
  const label = (action: GitAction) =>
    capitalized(view.git_actions.find((a) => a.action === action)?.label ?? action);
  return (
    <div className="flex flex-col gap-2 text-sm">
      <ReadList title="Protected paths" items={g.protected_paths} mono />
      <ReadList title="Protected git actions" items={g.protected_git.map(label)} />
      <ReadList title="Protected commands" items={g.protected_commands} mono />
      <p>
        <span className="font-medium">Member max:</span> {PRESET_NAMES[g.member_max_preset]}
      </p>
    </div>
  );
}

function ReadList(props: { title: string; items: string[]; mono?: boolean }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-0.5">
      <span id={id} className="font-medium">
        {props.title}
      </span>
      {props.items.length === 0 ? (
        <p className="text-xs text-muted-foreground">None.</p>
      ) : (
        <ul aria-labelledby={id} className="flex flex-wrap gap-1">
          {props.items.map((item) => (
            <li key={item}>
              <Badge variant="outline" className={props.mono ? "font-mono" : undefined}>
                {item}
              </Badge>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/// The form's state: on (with its fields as typed) or off.
type FormState =
  | { on: false }
  | { on: true; paths: string; git: GitAction[]; commands: string; max: Preset };

function formOf(g: Guardrails | null): FormState {
  return g === null
    ? { on: false }
    : {
        on: true,
        paths: g.protected_paths.join("\n"),
        git: g.protected_git,
        commands: g.protected_commands.join("\n"),
        max: g.member_max_preset,
      };
}

function guardrailsOf(form: FormState, order: GitAction[]): Guardrails | null {
  if (!form.on) return null;
  return {
    protected_paths: lines(form.paths),
    // In the order the core lists them.
    protected_git: order.filter((a) => form.git.includes(a)),
    protected_commands: lines(form.commands),
    member_max_preset: form.max,
  };
}

/// The editable guardrails: Use defaults and Turn off fill the form; Save signs what it holds
/// against the view's `seq`. The core checks the paths and commands and refuses with its words.
function GuardrailsForm(props: {
  view: GuardrailsView;
  lock: string | undefined;
  onSave: (guardrails: Guardrails | null) => void;
}) {
  const { view, lock } = props;
  const id = useId();
  const [form, setForm] = useState<FormState>(() => formOf(view.guardrails));
  const order = view.git_actions.map((a) => a.action);
  const result = guardrailsOf(form, order);
  const unchanged = JSON.stringify(result) === JSON.stringify(view.guardrails);
  const saveBlocker = lock ?? (unchanged ? "Nothing has changed." : undefined);
  const set = (change: Partial<Extract<FormState, { on: true }>>) =>
    setForm((f) => (f.on ? { ...f, ...change } : f));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!!lock}
          title={lock}
          onClick={() => setForm(formOf(view.defaults))}
        >
          Use defaults
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!!lock || !form.on}
          title={lock ?? (form.on ? undefined : "They are off.")}
          onClick={() => setForm({ on: false })}
        >
          Turn off
        </Button>
      </div>
      {form.on ? (
        <>
          <Field
            label="Protected paths"
            htmlFor={`${id}-paths`}
            hint="One glob a line, relative to the sol's folder: * stays in a folder, ** crosses folders."
          >
            <Textarea
              id={`${id}-paths`}
              className="font-mono text-xs"
              rows={6}
              disabled={!!lock}
              value={form.paths}
              onChange={(e) => set({ paths: e.target.value })}
            />
          </Field>
          <fieldset className="flex flex-col gap-1.5" disabled={!!lock}>
            <legend className="mb-1.5 text-sm font-medium">Protected git actions</legend>
            {view.git_actions.map(({ action, label }) => (
              <label key={action} className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={form.git.includes(action)}
                  onChange={(e) =>
                    set({
                      git: e.target.checked
                        ? [...form.git, action]
                        : form.git.filter((a) => a !== action),
                    })
                  }
                />
                {capitalized(label)}
              </label>
            ))}
          </fieldset>
          <Field
            label="Protected commands"
            htmlFor={`${id}-commands`}
            hint="One a line, like terraform apply: the command and words it must have, in any order."
          >
            <Textarea
              id={`${id}-commands`}
              className="font-mono text-xs"
              rows={6}
              disabled={!!lock}
              value={form.commands}
              onChange={(e) => set({ commands: e.target.value })}
            />
          </Field>
          <Field
            label="Member max"
            htmlFor={`${id}-max`}
            hint="The highest preset a Member's sol may use here."
          >
            <NativeSelect
              id={`${id}-max`}
              disabled={!!lock}
              value={form.max}
              onChange={(e) => set({ max: e.target.value as Preset })}
            >
              {MAX_PRESETS.map((p) => (
                <NativeSelectOption key={p} value={p}>
                  {PRESET_NAMES[p]}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </Field>
        </>
      ) : (
        <p className="text-sm text-muted-foreground">
          Off: nothing is protected in {view.workspace_name}. Use defaults to start from the usual
          set.
        </p>
      )}
      {result !== null && <p className="text-xs text-needs">{view.compat_note}</p>}
      <div className="flex justify-end">
        {saveBlocker ? (
          <Button type="button" size="sm" disabled title={saveBlocker}>
            Save
          </Button>
        ) : (
          <ConfirmButton
            variant="default"
            title={
              result === null
                ? `Turn off the guardrails of ${view.workspace_name}?`
                : `Sign the guardrails of ${view.workspace_name}?`
            }
            description={result === null ? OFF_TEXT : view.compat_note}
            confirmLabel="Sign and save"
            onConfirm={() => props.onSave(result)}
          >
            Save
          </ConfirmButton>
        )}
      </div>
    </div>
  );
}
