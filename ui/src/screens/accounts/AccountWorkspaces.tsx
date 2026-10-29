import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { OrgSummary, RosterEdit, UseRules, WorkspaceView } from "@/api/types";
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
import { keys } from "@/lib/query";

import { BUSY, DraftBanner, rosterBlockers, useRosterEdits } from "./AccountMembers";

/// The delete confirm's words: what happens to the workspace's dots.
export const DELETE_TEXT = "Its dots on each PC move to Default.";

/// Why an account with no member list has no workspace edits: they are signed into the list.
const LEGACY_TEXT = "This account has no member list, so its only workspace is Default.";

/// A workspace's `use` rules in words: they narrow the account's, and are changed with
/// dots-admin, so they are only shown here.
function useText(rules: UseRules | null): string {
  const parts = [];
  if (rules && rules.allow.length > 0) parts.push(`allow ${rules.allow.join(", ")}`);
  if (rules && rules.deny.length > 0) parts.push(`deny ${rules.deny.join(", ")}`);
  return `use: ${parts.length > 0 ? parts.join("; ") : "no extra rules"}`;
}

/// A name asked for in a small dialog: a new workspace's, or a new name for one.
function NameDialog(props: {
  title: string;
  confirm: string;
  initial: string;
  onSave: (name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(props.initial);
  const empty = name.trim() === "";
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle>{props.title}</DialogTitle>
          <DialogDescription>Its id is made from the name.</DialogDescription>
        </DialogHeader>
        <Field label="Name" htmlFor="workspace-name">
          <Input
            id="workspace-name"
            autoComplete="off"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={empty}
            title={empty ? "Give the workspace a name first." : undefined}
            onClick={() => {
              props.onSave(name.trim());
              props.onClose();
            }}
          >
            {props.confirm}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// A list of workspaces with Add workspace, Rename and Delete, for an account or Personal.
/// `lock` is why the three are off (undefined when they work); Default has no Delete.
export function WorkspaceList(props: {
  label: string;
  items: WorkspaceView[];
  lock: string | undefined;
  onAdd: (name: string) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}) {
  const [dialog, setDialog] = useState<"add" | WorkspaceView>();
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <p className="flex-1 text-xs text-muted-foreground">Workspaces in {props.label}</p>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!!props.lock}
          title={props.lock}
          onClick={() => setDialog("add")}
        >
          Add workspace
        </Button>
      </div>
      <ul aria-label={`Workspaces in ${props.label}`} className="flex flex-col gap-2">
        {props.items.map((w) => (
          <li
            key={w.id}
            aria-label={w.name}
            className="flex flex-wrap items-center gap-2 rounded-md border p-2"
          >
            <div className="flex min-w-0 flex-1 flex-col">
              <span className="flex items-center gap-2">
                <span className="font-medium">{w.name}</span>
                {w.id === "default" && <Badge variant="secondary">Default</Badge>}
              </span>
              <span className="text-xs text-muted-foreground">
                <span className="font-mono">{w.id}</span> · {useText(w.use_rules)}
              </span>
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={!!props.lock}
              title={props.lock}
              aria-label={`Rename ${w.name}`}
              onClick={() => setDialog(w)}
            >
              Rename
            </Button>
            {w.id !== "default" &&
              (props.lock ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled
                  title={props.lock}
                  aria-label={`Delete ${w.name}`}
                >
                  Delete
                </Button>
              ) : (
                <ConfirmButton
                  title={`Delete the ${w.name} workspace?`}
                  description={DELETE_TEXT}
                  confirmLabel="Delete"
                  ariaLabel={`Delete ${w.name}`}
                  onConfirm={() => props.onDelete(w.id)}
                >
                  Delete
                </ConfirmButton>
              ))}
          </li>
        ))}
      </ul>
      {dialog === "add" && (
        <NameDialog
          title="Add a workspace"
          confirm="Add"
          initial=""
          onSave={props.onAdd}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog && dialog !== "add" && (
        <NameDialog
          title={`Rename ${dialog.name}`}
          confirm="Rename"
          initial={dialog.name}
          onSave={(name) => props.onRename(dialog.id, name)}
          onClose={() => setDialog(undefined)}
        />
      )}
    </div>
  );
}

/// The Workspaces tab of an account (workspaces spec): its workspaces, and for an Admin or
/// Owner whose Dots ID signs, Add workspace, Rename and Delete as signed member-list edits.
/// They share the Members tab's draft, conflict and refusal handling. An account with no
/// member list (`legacy`) only shows Default.
export function AccountWorkspaces({ org }: { org: OrgSummary }) {
  const legacy = org.membership !== "active";
  const { list, notice, act } = useRosterEdits(org, !legacy);
  const spaces = useQuery({
    queryKey: keys.workspaces(org.id),
    queryFn: () => commands.workspaceList(org.id),
  });
  if (spaces.isError) return <ErrorNote error={spaces.error} />;
  if (list.isError) return <ErrorNote error={list.error} />;
  const items = spaces.data;
  if (!items) return <p className="text-xs text-muted-foreground">Loading the workspaces…</p>;
  const view = list.data;
  if (!legacy && !view) {
    return <p className="text-xs text-muted-foreground">Loading the member list…</p>;
  }

  const busy = act.isPending;
  const blockers = view ? rosterBlockers(org, view) : undefined;
  const refused = items.some((w) => w.can_manage)
    ? undefined
    : `${org.name}: you can't change its workspaces. Ask an Admin or Owner.`;
  const lock = legacy
    ? LEGACY_TEXT
    : (blockers?.editBlocker ?? refused ?? (busy ? BUSY : undefined));
  const send = (edit: RosterEdit) => {
    if (view) act.mutate(() => commands.memberEdit(org.id, view.seq, edit));
  };
  return (
    <div className="flex flex-col gap-3 pt-1">
      {view?.draft && blockers && (
        <DraftBanner
          view={view}
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
      <WorkspaceList
        label={org.name}
        items={items}
        lock={lock}
        onAdd={(name) => send({ kind: "add_workspace", name })}
        onRename={(id, name) => send({ kind: "rename_workspace", id, name })}
        onDelete={(id) => send({ kind: "delete_workspace", id })}
      />
    </div>
  );
}
