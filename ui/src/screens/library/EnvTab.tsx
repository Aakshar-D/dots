import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { EnvEntry } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Field } from "@/components/Field";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ItemRow } from "./parts";

/// A variable name as `vault::entries::check_variable_name` takes it.
export const validVariableName = (name: string) => /^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(name);
const NAME_RULE = "Use a letter or _, then letters, digits or _ (at most 128).";

/// Add a variable (`entry` null), or edit a plain one's value or replace a secret's. A secret's
/// value is typed into a password field and never shown, before or after it is saved.
function VariableDialog(props: { entry: EnvEntry | null; onClose: () => void }) {
  const qc = useQueryClient();
  const { entry } = props;
  const [name, setName] = useState(entry?.name ?? "");
  const [secret, setSecret] = useState(entry ? entry.kind === "secret" : true);
  const [value, setValue] = useState(entry?.kind === "plain" ? (entry.value ?? "") : "");
  const [tried, setTried] = useState(false);
  const [state, setState] = useState<{ busy: boolean; error?: string }>({ busy: false });
  const errors = {
    name: entry || validVariableName(name) ? undefined : NAME_RULE,
    value: secret && value === "" ? "A secret needs a value." : undefined,
  };

  async function save() {
    setTried(true);
    if (errors.name || errors.value) return;
    setState({ busy: true });
    try {
      if (entry) await commands.envSet(entry.name, value);
      else await commands.envAdd(name, secret ? "secret" : "plain", value);
      invalidate(qc, keys.env);
      invalidate(qc, keys.library);
      props.onClose();
    } catch (e) {
      setState({ busy: false, error: errorMessage(e) });
    }
  }

  const title = !entry
    ? "Add variable"
    : entry.kind === "secret"
      ? `Replace ${entry.name}`
      : `Edit ${entry.name}`;
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            {entry?.kind === "secret"
              ? "The new value takes the old one's place. Neither is shown."
              : "A dot can read it only where it is granted it."}
          </DialogDescription>
        </DialogHeader>
        {!entry && (
          <>
            <Field label="Name" htmlFor="env-name" error={tried ? errors.name : undefined}>
              <Input
                id="env-name"
                className="font-mono"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <div className="flex items-center gap-2">
              <Switch id="env-secret" checked={secret} onCheckedChange={setSecret} />
              <Label htmlFor="env-secret">Secret</Label>
              <span className="text-xs text-muted-foreground">
                {secret
                  ? "Kept in Windows Credential Manager and never shown again."
                  : "Plain config: shown here and editable."}
              </span>
            </div>
          </>
        )}
        <Field label="Value" htmlFor="env-value" error={tried ? errors.value : undefined}>
          <Input
            id="env-value"
            type={secret ? "password" : "text"}
            autoComplete="off"
            className="font-mono"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        </Field>
        {state.error && <p className="text-sm text-destructive">{state.error}</p>}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={state.busy} onClick={() => void save()}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RemoveVariable({ entry }: { entry: EnvEntry }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.envRemove(entry.name),
    onSuccess: () => {
      invalidate(qc, keys.env);
      invalidate(qc, keys.library);
    },
    onError: notifyError,
  });
  return (
    <ConfirmButton
      title={`Remove ${entry.name}?`}
      description="A dot that is granted it can no longer read it."
      confirmLabel="Remove variable"
      disabled={remove.isPending}
      onConfirm={() => remove.mutate()}
    >
      Remove
    </ConfirmButton>
  );
}

/// The one-time offer to copy the variables Library servers read from Dots' Windows
/// environment, which Dots no longer reads.
function ImportOffer({ names }: { names: string[] }) {
  const qc = useQueryClient();
  const answer = useMutation({
    mutationFn: (take: boolean) => (take ? commands.envImport(names) : commands.envImportDismiss()),
    onSuccess: () => {
      invalidate(qc, keys.env);
      invalidate(qc, keys.envImport);
      invalidate(qc, keys.library);
    },
    onError: notifyError,
  });
  return (
    <div role="region" aria-label="Import" className="flex flex-col gap-2 rounded-lg border p-3">
      <p className="text-sm">
        Dots no longer reads its own Windows environment. Library servers use these variables,
        which Windows sets: <span className="font-mono">{names.join(", ")}</span>. Import them
        into Personal as secrets?
      </p>
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          disabled={answer.isPending}
          onClick={() => answer.mutate(true)}
        >
          Import
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          Don't import
        </Button>
      </div>
    </div>
  );
}

/// The one-time offer to delete the update backups made before the vault: they may hold keys
/// in plain text. Dots never deletes them on its own.
function BackupsOffer({ files }: { files: string[] }) {
  const qc = useQueryClient();
  // The backups the last "Delete backups" could not delete: the offer stays for them.
  const [stuck, setStuck] = useState<string[]>([]);
  const answer = useMutation({
    mutationFn: (remove: boolean) => commands.envOldBackupsAnswer(remove),
    onSuccess: (failed) => {
      setStuck(failed);
      invalidate(qc, keys.envBackups);
    },
    onError: notifyError,
  });
  return (
    <div
      role="region"
      aria-label="Old backups"
      className="flex flex-col gap-2 rounded-lg border p-3"
    >
      <p className="text-sm">
        These update backups were made before the vault and may hold keys in plain text:{" "}
        <span className="font-mono">{files.join(", ")}</span>. Delete them?
      </p>
      {stuck.length > 0 && (
        <p role="alert" className="text-sm text-destructive">
          Could not delete {stuck.join(", ")}. Close any program that has it open, then try
          again.
        </p>
      )}
      <div className="flex gap-2">
        <ConfirmButton
          title="Delete the old backups?"
          description="They cannot be brought back."
          confirmLabel="Delete backups"
          variant="destructive"
          disabled={answer.isPending}
          onConfirm={() => answer.mutate(true)}
        >
          Delete backups
        </ConfirmButton>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          Keep them
        </Button>
      </div>
    </div>
  );
}

/// The Library's Env tab (env vault spec §9): the personal variables a dot can be granted. A
/// secret's value is never shown.
export function EnvTab() {
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const imports = useQuery({ queryKey: keys.envImport, queryFn: commands.envImportCandidates });
  const backups = useQuery({ queryKey: keys.envBackups, queryFn: commands.envOldBackups });
  const [editing, setEditing] = useState<EnvEntry | null | undefined>(undefined);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-4">
        <p className="text-sm text-muted-foreground">
          Variables your dots can be granted. A server's <code>{"${VAR}"}</code> reads them. A
          secret is kept in Windows Credential Manager and never shown again.
        </p>
        <Button type="button" onClick={() => setEditing(null)}>
          <Plus aria-hidden /> Add variable
        </Button>
      </div>
      {imports.data && imports.data.length > 0 && <ImportOffer names={imports.data} />}
      {backups.data && backups.data.length > 0 && <BackupsOffer files={backups.data} />}
      <section aria-label="Personal" className="flex flex-col gap-2">
        <h2 className="text-sm font-medium">Personal</h2>
        {env.isError && <ErrorNote error={env.error} />}
        {env.data?.length === 0 && (
          <p className="text-sm text-muted-foreground">No variables yet.</p>
        )}
        <ul className="flex flex-col gap-2">
          {(env.data ?? []).map((entry) => (
            <ItemRow
              key={entry.name}
              label={entry.name}
              actions={
                <>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => setEditing(entry)}
                  >
                    {entry.kind === "secret" ? "Replace value" : "Edit"}
                  </Button>
                  <RemoveVariable entry={entry} />
                </>
              }
            >
              <div className="flex items-center gap-2">
                <span className="font-mono text-sm font-medium">{entry.name}</span>
                <Badge variant="secondary">{entry.kind}</Badge>
              </div>
              {entry.kind === "secret" ? (
                <span className="text-xs text-muted-foreground">Kept in Credential Manager</span>
              ) : (
                <span className="truncate font-mono text-xs text-muted-foreground">
                  {entry.value}
                </span>
              )}
            </ItemRow>
          ))}
        </ul>
      </section>
      {editing !== undefined && (
        <VariableDialog entry={editing} onClose={() => setEditing(undefined)} />
      )}
    </div>
  );
}
