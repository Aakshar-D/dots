import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, ChevronRight, Target } from "lucide-react";
import { useId, useState } from "react";
import { useNavigate } from "react-router";

import { commands } from "@/api/commands";
import type { Dot, Intent, IntentFields } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { groupIntents } from "@/lib/intents";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { PERSONAL, useCurrentScope } from "@/lib/scope";
import { onSingleClick } from "@/lib/single-click";

/// The longest goal, in characters after trimming, as the core counts them.
const MAX_TEXT = 500;

/// What the router carries to the home when Start opens a dot: the first message's draft.
export type StartDraft = { draft: string };

const blank: IntentFields = { text: "", due_date: null, urgent: false, dot_id: null };

/// "Fri 7 Nov" for a due date, in the person's locale.
function dueLabel(date: string): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    weekday: "short",
    day: "numeric",
    month: "short",
  });
}

/// The fields of a goal: its text, then an optional due date, Urgent and a dot of this
/// workspace. The quick-add box at the top and the Edit dialog both use it.
function GoalFields(props: {
  value: IntentFields;
  onChange: (v: IntentFields) => void;
  dots: Dot[];
  idPrefix: string;
  autoFocus?: boolean;
  onEnter?: () => void;
}) {
  const { value, onChange, idPrefix } = props;
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Input
        aria-label="Goal"
        placeholder="Add a goal…"
        autoComplete="off"
        autoFocus={props.autoFocus}
        className="min-w-48 flex-1"
        value={value.text}
        onChange={(e) => onChange({ ...value, text: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === "Enter" && props.onEnter) {
            e.preventDefault();
            props.onEnter();
          }
        }}
      />
      <Input
        type="date"
        aria-label="Due date"
        className="w-40"
        value={value.due_date ?? ""}
        onChange={(e) => onChange({ ...value, due_date: e.target.value || null })}
      />
      <label htmlFor={`${idPrefix}-urgent`} className="flex items-center gap-1.5 text-sm">
        <Switch
          id={`${idPrefix}-urgent`}
          checked={value.urgent}
          onCheckedChange={(urgent) => onChange({ ...value, urgent })}
        />
        Urgent
      </label>
      <NativeSelect
        aria-label="Dot"
        value={value.dot_id ?? ""}
        onChange={(e) => onChange({ ...value, dot_id: e.target.value || null })}
      >
        <NativeSelectOption value="">No dot</NativeSelectOption>
        {props.dots.map((dot) => (
          <NativeSelectOption key={dot.id} value={dot.id}>
            {dot.name}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </div>
  );
}

/// Whether `fields` can be saved: a text of 1 to 500 characters.
function fits(fields: IntentFields): boolean {
  const n = [...fields.text.trim()].length;
  return n > 0 && n <= MAX_TEXT;
}

/// The small dialog a goal is edited in. Closes once the core has saved it; a refusal stays
/// in the dialog, in the core's words.
function EditDialog(props: {
  intent: Intent;
  dots: Dot[];
  onClose: () => void;
  refresh: () => void;
}) {
  const { intent } = props;
  const [value, setValue] = useState<IntentFields>({
    text: intent.text,
    due_date: intent.due_date,
    urgent: intent.urgent,
    dot_id: intent.dot_id,
  });
  const save = useMutation({
    mutationFn: (fields: IntentFields) => commands.intentUpdate(intent.id, fields),
    onSuccess: props.onClose,
    onSettled: props.refresh,
  });
  const id = useId();
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-xl" aria-describedby={undefined}>
        <DialogHeader>
          <DialogTitle>Edit goal</DialogTitle>
        </DialogHeader>
        <GoalFields
          value={value}
          onChange={setValue}
          dots={props.dots}
          idPrefix={id}
          autoFocus
          onEnter={() => fits(value) && save.mutate(value)}
        />
        {save.error !== null && (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(save.error)}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!fits(value) || save.isPending}
            onClick={onSingleClick(() => save.mutate(value))}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// One goal: its checkbox, its text with the due date and dot, and (when the person may change
/// goals) Start for a goal handed to a dot, Edit and Delete.
function GoalRow(props: {
  intent: Intent;
  dot: Dot | undefined;
  readOnly: boolean;
  onDone: (done: boolean) => void;
  onEdit: () => void;
  onDelete: () => void;
  onStart: () => void;
}) {
  const { intent, dot, readOnly } = props;
  const done = intent.done_at !== null;
  const boxId = useId();
  return (
    <li aria-label={intent.text} className="flex items-center gap-3 rounded-lg border px-3 py-2">
      <input
        id={boxId}
        type="checkbox"
        className="size-4 shrink-0 accent-primary"
        aria-label={done ? `Mark "${intent.text}" not done` : `Mark "${intent.text}" done`}
        checked={done}
        disabled={readOnly}
        onChange={(e) => props.onDone(e.target.checked)}
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className={done ? "text-muted-foreground line-through" : undefined}>
          {intent.text}
        </span>
        {(intent.due_date !== null || dot !== undefined) && (
          <span className="text-xs text-muted-foreground">
            {[intent.due_date !== null && `Due ${dueLabel(intent.due_date)}`, dot?.name]
              .filter(Boolean)
              .join(" · ")}
          </span>
        )}
      </div>
      {!readOnly && (
        <>
          {dot !== undefined && !done && (
            <Button size="sm" onClick={props.onStart} title={`Start a new topic with ${dot.name}`}>
              Start
            </Button>
          )}
          <Button size="sm" variant="outline" onClick={props.onEdit}>
            Edit
          </Button>
          <ConfirmButton
            title="Delete this goal?"
            description={intent.text}
            confirmLabel="Delete"
            onConfirm={props.onDelete}
          >
            Delete
          </ConfirmButton>
        </>
      )}
    </li>
  );
}

/// The Intents screen: the person's goals in the account and workspace they work in, grouped
/// by how soon they matter, with a quick-add box at the top and the done ones at the bottom.
/// The core announces every change (`intents_updated`), so the list refreshes itself.
export function IntentsScreen() {
  const scope = useCurrentScope();
  return (
    <Page title="Intents">
      <p className="-mt-2 text-muted-foreground">Goals, grouped by how soon they matter.</p>
      {scope && <Goals account={scope.account} workspace={scope.workspace} />}
    </Page>
  );
}

function Goals({ account, workspace }: { account: string; workspace: string }) {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const key = keys.intents(account, workspace);
  const list = useQuery({
    queryKey: key,
    queryFn: () => commands.intentsList(account, workspace),
  });
  const overview = useQuery({ queryKey: keys.overview, queryFn: commands.dotOverview }).data;
  // The dots a goal may be handed to: those of this account and workspace.
  const dots = (overview ?? [])
    .map((row) => row.dot)
    .filter((d) => (d.org_id ?? PERSONAL) === account && d.workspace_id === workspace);
  const [draft, setDraft] = useState<IntentFields>(blank);
  const [editing, setEditing] = useState<Intent | null>(null);
  const [showDone, setShowDone] = useState(false);
  const addId = useId();
  const refresh = () => invalidate(qc, key);
  const add = useMutation({
    mutationFn: (fields: IntentFields) => commands.intentCreate(account, workspace, fields),
    onSuccess: () => setDraft(blank),
    onSettled: refresh,
  });
  const setDone = useMutation({
    mutationFn: (v: { id: string; done: boolean }) => commands.intentSetDone(v.id, v.done),
    onError: notifyError,
    onSettled: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: string) => commands.intentDelete(id),
    onError: notifyError,
    onSettled: refresh,
  });

  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  const { intents, refusal } = list.data;
  const readOnly = refusal !== null;
  const { groups, done } = groupIntents(intents);
  const submit = () => {
    if (fits(draft) && !add.isPending) add.mutate(draft);
  };
  const row = (intent: Intent) => (
    <GoalRow
      key={intent.id}
      intent={intent}
      dot={dots.find((d) => d.id === intent.dot_id)}
      readOnly={readOnly}
      onDone={(isDone) => setDone.mutate({ id: intent.id, done: isDone })}
      onEdit={() => setEditing(intent)}
      onDelete={() => remove.mutate(intent.id)}
      onStart={() =>
        void navigate(`/home/${intent.dot_id}`, {
          state: { draft: intent.text } satisfies StartDraft,
        })
      }
    />
  );
  return (
    <div className="flex flex-col gap-4">
      {readOnly ? (
        <p role="status" className="text-muted-foreground">
          {refusal}
        </p>
      ) : (
        <form
          aria-label="Add a goal"
          className="flex flex-col gap-2 rounded-lg border p-3"
          onSubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <GoalFields
            value={draft}
            onChange={setDraft}
            dots={dots}
            idPrefix={addId}
            onEnter={submit}
          />
          <div className="flex items-center gap-2">
            {add.error !== null ? (
              <p role="alert" className="flex-1 text-sm text-destructive">
                {errorMessage(add.error)}
              </p>
            ) : (
              <span className="flex-1" />
            )}
            <Button type="submit" size="sm" disabled={!fits(draft) || add.isPending}>
              Add
            </Button>
          </div>
        </form>
      )}
      {intents.length === 0 ? (
        <EmptyState icon={Target}>
          {readOnly
            ? "No goals here yet."
            : "No goals yet. Add one above, with a date if it has one."}
        </EmptyState>
      ) : (
        <>
          {groups.map((group) => (
            <section key={group.key} aria-label={group.label} className="flex flex-col gap-2">
              <h2 className="text-sm font-semibold">{group.label}</h2>
              <ul className="flex flex-col gap-2">{group.intents.map(row)}</ul>
            </section>
          ))}
          {done.length > 0 && (
            <section aria-label="Done" className="flex flex-col gap-2">
              <Button
                variant="ghost"
                size="sm"
                className="self-start"
                aria-expanded={showDone}
                onClick={() => setShowDone(!showDone)}
              >
                {showDone ? <ChevronDown aria-hidden /> : <ChevronRight aria-hidden />}
                Done ({done.length})
              </Button>
              {showDone && <ul className="flex flex-col gap-2">{done.map(row)}</ul>}
            </section>
          )}
        </>
      )}
      {editing && (
        <EditDialog
          intent={editing}
          dots={dots}
          onClose={() => setEditing(null)}
          refresh={refresh}
        />
      )}
    </div>
  );
}
