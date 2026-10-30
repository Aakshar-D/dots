import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { ApiKeyView, WorkspaceView } from "@/api/types";
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
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// The revoke confirm's words.
export const REVOKE_TEXT = "Apps using this key stop working at once.";

/// The command for the sample: `curl.exe` on Windows, where PowerShell's `curl` is another command.
function curlCommand(): string {
  return /windows/i.test(navigator.userAgent) ? "curl.exe" : "curl";
}

/// What the create answer shows: the key, and where to call or why that is not known.
type Made = { secret: string; base: { url: string } | { reason: string } };

/// Makes a key: a name and a workspace first, then the key itself, shown once. The key lives
/// in this dialog's state alone, never in the query cache, so closing the dialog drops it.
function CreateDialog(props: { scope: string; onClose: () => void }) {
  const qc = useQueryClient();
  const spaces = useQuery({
    queryKey: keys.workspaces(props.scope),
    queryFn: () => commands.workspaceList(props.scope),
  });
  const [name, setName] = useState("");
  const [picked, setPicked] = useState<string>();
  const [made, setMade] = useState<Made>();
  const items: WorkspaceView[] = spaces.data ?? [];
  const workspace = picked ?? items.find((w) => !w.refusals.create_api_keys)?.id ?? "";
  const refusal = items.find((w) => w.id === workspace)?.refusals.create_api_keys ?? null;
  // The key goes straight into this dialog's state: the mutation below returns nothing, so the
  // mutation cache (kept for minutes after the dialog closes) never holds it.
  const create = useMutation({
    mutationFn: async (): Promise<void> => {
      const answer = await commands.apiKeyCreate(props.scope, workspace, name.trim());
      // The server may be off: its reason is shown in the core's words beside the key.
      const base = await commands.apiBaseUrl().then(
        (url) => ({ url }),
        (e: unknown) => ({ reason: errorMessage(e) }),
      );
      setMade({ secret: answer.secret, base });
    },
    onSuccess: () => {
      invalidate(qc, keys.apiKeys(props.scope));
      invalidate(qc, keys.workspaces(props.scope));
    },
    onError: notifyError,
  });
  const noneAllowed = items.length > 0 && items.every((w) => w.refusals.create_api_keys);
  const why = spaces.isError
    ? "The workspaces could not be loaded."
    : !spaces.data
      ? "Loading workspaces…"
      : name.trim() === ""
        ? "Give the key a name first."
        : noneAllowed
          ? (items[0]?.refusals.create_api_keys ?? null)
          : refusal;
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{made ? "Your new API key" : "Create an API key"}</DialogTitle>
          <DialogDescription>
            {made
              ? "Copy it now. Dots won't show it again."
              : "A key lets an app list and run the dots of one workspace."}
          </DialogDescription>
        </DialogHeader>
        {made ? (
          <Secret made={made} />
        ) : (
          <>
            {spaces.isError && <ErrorNote error={spaces.error} />}
            {!spaces.data && !spaces.isError && (
              <p className="text-xs text-muted-foreground">Loading workspaces…</p>
            )}
            {noneAllowed && (
              <p className="text-xs text-muted-foreground">{items[0]?.refusals.create_api_keys}</p>
            )}
            <Field label="Name" htmlFor="api-key-name">
              <Input
                id="api-key-name"
                autoComplete="off"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <Field
              label="Workspace"
              htmlFor="api-key-workspace"
              {...(refusal ? { error: refusal } : {})}
            >
              <NativeSelect
                id="api-key-workspace"
                value={workspace}
                onChange={(e) => setPicked(e.target.value)}
              >
                {items.map((w) => (
                  <NativeSelectOption
                    key={w.id}
                    value={w.id}
                    disabled={!!w.refusals.create_api_keys}
                    title={w.refusals.create_api_keys ?? undefined}
                  >
                    {w.name}
                  </NativeSelectOption>
                ))}
              </NativeSelect>
            </Field>
          </>
        )}
        <DialogFooter>
          {made ? (
            <Button type="button" onClick={props.onClose}>
              Done
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={props.onClose}>
                Cancel
              </Button>
              <Button
                type="button"
                disabled={!!why || workspace === "" || create.isPending}
                title={why ?? undefined}
                onClick={() => create.mutate()}
              >
                Create
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The key in a box to read and copy, and how to call the server with it.
function Secret({ made }: { made: Made }) {
  const url = "url" in made.base ? made.base.url : undefined;
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <Input readOnly aria-label="API key" className="font-mono text-xs" value={made.secret} />
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={() =>
            navigator.clipboard
              .writeText(made.secret)
              .then(() => toast.success("Copied the API key"), notifyError)
          }
        >
          <Copy aria-hidden /> Copy
        </Button>
      </div>
      {"url" in made.base ? (
        <p className="text-xs text-muted-foreground">
          Base URL: <span className="font-mono">{made.base.url}</span>
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">{made.base.reason}</p>
      )}
      {url && (
        <code className="block overflow-x-auto rounded-md border p-2 font-mono text-xs whitespace-pre">
          {`${curlCommand()} -H "Authorization: Bearer ${made.secret}" ${url}/dots`}
        </code>
      )}
    </div>
  );
}

function KeyRow({ k, onRevoke }: { k: ApiKeyView; onRevoke: () => void }) {
  return (
    <li aria-label={k.name} className="flex flex-wrap items-center gap-2 rounded-md border p-2">
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex items-center gap-2">
          <span className="font-medium">{k.name}</span>
          {k.revoked_at && <Badge variant="secondary">Revoked</Badge>}
        </span>
        <span className="text-xs text-muted-foreground">
          <span className="font-mono">{k.prefix}…</span> · {k.workspace_name} · Created{" "}
          {localDateTime(k.created_at)} · Last used{" "}
          {k.last_used_at ? localDateTime(k.last_used_at) : "Never"}
        </span>
      </div>
      {/* A revoked key cannot be used again, so there is nothing left to revoke. */}
      {/* Revoking only reduces access, so any role may: `refusal` is about making keys. */}
      {!k.revoked_at && (
        <ConfirmButton
          title={`Revoke ${k.name}?`}
          description={REVOKE_TEXT}
          confirmLabel="Revoke"
          ariaLabel={`Revoke ${k.name}`}
          onConfirm={onRevoke}
        >
          Revoke
        </ConfirmButton>
      )}
    </li>
  );
}

/// The API keys of an account (`scope` is its id) or of Personal (`personal`): the list, Create
/// key (the key is shown once) and Revoke. Making a key their role does not allow is off with the
/// core's reason; revoking is always allowed.
export function ApiKeys({ scope }: { scope: string }) {
  const qc = useQueryClient();
  const [creating, setCreating] = useState(false);
  // Asked again each time the list is shown: a key's use is not announced, so "Last used"
  // would otherwise stay as it was when the list was first loaded.
  const list = useQuery({
    queryKey: keys.apiKeys(scope),
    queryFn: () => commands.apiKeyList(scope),
    refetchOnMount: "always",
  });
  const revoke = useMutation({
    mutationFn: (id: string) => commands.apiKeyRevoke(id),
    onSuccess: () => {
      invalidate(qc, keys.apiKeys(scope));
      invalidate(qc, keys.workspaces(scope));
    },
    onError: notifyError,
  });
  if (list.isError) return <ErrorNote error={list.error} />;
  if (!list.data) return <p className="text-xs text-muted-foreground">Loading the API keys…</p>;
  return (
    <div className="flex flex-col gap-2 pt-1">
      <div className="flex items-center gap-2">
        <p className="flex-1 text-xs text-muted-foreground">
          Keys let an app list and run dots through the local server.
        </p>
        <Button type="button" size="sm" variant="outline" onClick={() => setCreating(true)}>
          Create key
        </Button>
      </div>
      {list.data.length === 0 ? (
        <p className="text-sm text-muted-foreground">No API key yet.</p>
      ) : (
        <ul aria-label="API keys" className="flex flex-col gap-2">
          {list.data.map((k) => (
            <KeyRow key={k.id} k={k} onRevoke={() => revoke.mutate(k.id)} />
          ))}
        </ul>
      )}
      {creating && <CreateDialog scope={scope} onClose={() => setCreating(false)} />}
    </div>
  );
}
