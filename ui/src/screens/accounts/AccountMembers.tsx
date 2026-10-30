import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type {
  EditOutcome,
  MembersView,
  MemberView,
  OrgSummary,
  Role,
  WorkspaceRole,
  RosterEdit,
} from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { ErrorNote } from "@/components/ErrorNote";
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
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { accessOf, notAllowedText, roleLabel } from "@/lib/access";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { onSingleClick } from "@/lib/single-click";

import { ApproveDialog } from "./ApproveDialog";
import { INVITE_ROLES, InviteDialog } from "./InviteDialog";

/// The core's refusal for an edit that touches an Owner (`accounts::edit::OWNER_LOCKED`).
export const OWNER_TEXT =
  "Owners can only be changed with the account owner's key, using dots-admin.";

/// An Admin whose Dots ID isn't in the policy's `admins` (members spec §6.1).
const NOT_SIGNING = "Can sign the list once the owner adds their Dots ID to the policy's admins.";

/// Why a control is off for the moment while the last change is sent.
export const BUSY = "Waiting for the last change to finish.";

/// Why an edit waits: the core takes one change at a time while a draft waits (accounts spec
/// §6.5), and refuses a new one with these words.
export const DRAFT_WAITING = "a change is waiting to be published. Retry or discard it first.";

/// The roles a member may hold in one workspace.
const WORKSPACE_ROLES: WorkspaceRole[] = ["admin", "member", "viewer"];

const OWN_ROLE = "You're changing your own role. You may lose the right to manage members.";

const REMOVE_PC_TEXT =
  "Also revoke its access in the source: the list stops Dots honouring this PC, only the " +
  "source cuts it off.";

const REMOVE_TEXT =
  "Also revoke their access in the source: the list stops Dots honouring them, only the " +
  "source cuts them off.";

/// What the member-list edits of an account share (Members and Workspaces tabs): the list, the
/// core's words for the last edit that failed, and one change at a time. What changed the list
/// asks for it, the org's row and the workspaces again. A refusal does too: the list may have
/// changed (a `conflict`) or its draft been dropped.
export function useRosterEdits(org: OrgSummary, enabled = true) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.orgMembers(org.id),
    queryFn: () => commands.memberList(org.id),
    enabled,
  });
  // The core's words for the last edit that failed.
  const [notice, setNotice] = useState<string>();
  const changed = () => {
    invalidate(qc, keys.orgs);
    invalidate(qc, ["workspaces"]);
  };
  const told = (outcome: EditOutcome | null) => {
    if (outcome?.outcome === "published") toast.success(`Published the member list of ${org.name}`);
  };
  const act = useMutation({
    mutationFn: (task: () => Promise<EditOutcome | null>) => task(),
    onMutate: () => setNotice(undefined),
    onSuccess: (outcome) => {
      changed();
      told(outcome);
    },
    onError: (e) => {
      setNotice(errorMessage(e));
      changed();
    },
  });
  return { list, notice, changed, told, act };
}

/// Why the list can't be changed from this PC (`blocker`), why a change that needs the list
/// to have no draft can't be made (`editBlocker`), and why a draft can't be discarded
/// (`manageBlocker`); each undefined when it can.
export function rosterBlockers(org: OrgSummary, view: MembersView) {
  const manageBlocker = view.can_manage
    ? undefined
    : notAllowedText(org.name, accessOf(org, org.id), "members");
  const blocker =
    manageBlocker ??
    (view.can_sign
      ? undefined
      : (view.sign_blocker ?? `${org.name}: this PC's Dots ID can't sign the member list.`));
  // An approval, a role change or a removal also waits for a draft to be published or
  // discarded; inviting and revoking don't touch the list.
  const editBlocker = blocker ?? (view.draft ? `${org.name}: ${DRAFT_WAITING}` : undefined);
  return { manageBlocker, blocker, editBlocker };
}

/// The Members tab of an account (accounts spec §6.5, §7; members spec §6.2): who is on the
/// list and with which PCs, its open invites, and, for an Admin or Owner whose Dots ID signs,
/// Invite, Approve request, role changes and removals. Every edit is made against the list's
/// `seq`; a list that changed meanwhile is refused with the core's words and asked for again.
export function AccountMembers({ org }: { org: OrgSummary }) {
  const { list, notice, changed, told, act } = useRosterEdits(org);
  const [dialog, setDialog] = useState<"invite" | "approve">();

  if (list.isError) return <ErrorNote error={list.error} />;
  const view = list.data;
  if (!view) return <p className="text-xs text-muted-foreground">Loading the member list…</p>;

  const edit = (e: RosterEdit) => () => commands.memberEdit(org.id, view.seq, e);
  const { manageBlocker, blocker, editBlocker } = rosterBlockers(org, view);
  const busy = act.isPending;

  // An approval from the dialog: its error shows in the dialog, not here.
  async function approve(e: RosterEdit) {
    try {
      const outcome = await edit(e)();
      changed();
      told(outcome);
    } catch (err) {
      changed();
      throw err;
    }
  }

  const open = view.invites.filter((i) => i.state === "open");
  return (
    <div className="flex flex-col gap-3 pt-1">
      <div className="flex flex-wrap items-center gap-2">
        <p className="flex-1 text-xs text-muted-foreground">Member list #{view.seq}</p>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!!blocker}
          title={blocker}
          onClick={() => setDialog("invite")}
        >
          Invite
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!!editBlocker}
          title={editBlocker}
          onClick={() => setDialog("approve")}
        >
          Approve request
        </Button>
      </div>

      {view.draft && (
        <DraftBanner
          view={view}
          blocker={blocker}
          discardBlocker={manageBlocker}
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

      <ul aria-label="Members" className="flex flex-col gap-2">
        {view.members.map((m) => (
          <MemberRow
            key={m.id}
            member={m}
            account={org.name}
            orgId={org.id}
            lock={m.owner_locked ? OWNER_TEXT : editBlocker}
            busy={busy}
            onEdit={(e) => act.mutate(edit(e))}
          />
        ))}
      </ul>

      {open.length > 0 && (
        <div className="flex flex-col gap-1">
          <p className="text-xs font-medium">Open invites</p>
          <ul aria-label="Open invites" className="flex flex-col gap-1 text-xs">
            {open.map((invite) => (
              <li key={invite.id} className="flex items-center gap-2">
                <span className="flex-1">
                  {invite.email} · {roleLabel(invite.role)} · expires{" "}
                  {localDateTime(invite.expires_at)}
                </span>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={busy || !!blocker}
                  title={blocker ?? (busy ? BUSY : undefined)}
                  onClick={() =>
                    act.mutate(() => commands.inviteRevoke(org.id, invite.id).then(() => null))
                  }
                >
                  Revoke
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {dialog === "invite" && (
        <InviteDialog orgId={org.id} account={org.name} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "approve" && (
        <ApproveDialog
          orgId={org.id}
          account={org.name}
          approve={approve}
          onClose={() => setDialog(undefined)}
        />
      )}
    </div>
  );
}

/// One member: who, their role and workspaces, and each PC (Dots ID) on the list, with the
/// controls `lock` (why they are off) allows. A change that can take this PC's own rights away
/// (its own role, or removing it) is asked about first.
function MemberRow(props: {
  member: MemberView;
  account: string;
  orgId: string;
  lock: string | undefined;
  busy: boolean;
  onEdit: (edit: RosterEdit) => void;
}) {
  const { member: m, lock } = props;
  // The role picked for this PC's own member, waiting for the confirm.
  const [ownRole, setOwnRole] = useState<Role>();
  const [picking, setPicking] = useState(false);
  const isMe = m.keys.some((k) => k.is_me);
  const off = props.busy || !!lock;
  const why = lock ?? (props.busy ? BUSY : undefined);
  const workspaces = Object.entries(m.workspaces);
  const roles: Role[] = m.role === "owner" ? ["owner", ...INVITE_ROLES] : INVITE_ROLES;
  return (
    <li aria-label={m.name} className="flex flex-col gap-1.5 rounded-md border p-2">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="flex items-center gap-2">
            <span className="font-medium">{m.name}</span>
            <Badge variant="outline">{roleLabel(m.role)}</Badge>
          </span>
          <span className="text-xs text-muted-foreground">{m.email}</span>
          {workspaces.length > 0 && (
            <span className="text-xs text-muted-foreground">
              {workspaces.map(([ws, role]) => `${ws}: ${roleLabel(role)}`).join(", ")}
            </span>
          )}
        </div>
        <NativeSelect
          size="sm"
          aria-label={`Role of ${m.name}`}
          value={m.role}
          disabled={off}
          title={why}
          onChange={(e) => {
            const role = e.target.value as Role;
            if (isMe) setOwnRole(role);
            else props.onEdit({ kind: "set_role", member_id: m.id, role });
          }}
        >
          {roles.map((r) => (
            <NativeSelectOption key={r} value={r} disabled={r === "owner"}>
              {roleLabel(r)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={off}
          title={why}
          aria-label={`Workspaces of ${m.name}`}
          onClick={() => setPicking(true)}
        >
          Workspaces
        </Button>
        {off ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled
            title={why}
            aria-label={`Remove ${m.name}`}
          >
            Remove
          </Button>
        ) : (
          <ConfirmButton
            title={`Remove ${m.name}?`}
            description={REMOVE_TEXT}
            confirmLabel="Remove"
            ariaLabel={`Remove ${m.name}`}
            onConfirm={() => props.onEdit({ kind: "remove", member_id: m.id })}
          >
            Remove
          </ConfirmButton>
        )}
      </div>
      {m.role === "admin" && !m.keys.some((k) => k.signs) && (
        <p className="text-xs text-needs">{NOT_SIGNING}</p>
      )}
      <ul aria-label={`PCs of ${m.name}`} className="flex flex-col gap-1 text-xs">
        {m.keys.map((k) => (
          <li key={k.key} className="flex items-center gap-2">
            <span className="flex-1 font-mono">{k.fingerprint}</span>
            {k.is_me && <Badge variant="secondary">This PC</Badge>}
            {off ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled
                title={why}
                aria-label={`Remove this PC ${k.fingerprint}`}
              >
                Remove this PC
              </Button>
            ) : (
              <ConfirmButton
                variant="ghost"
                title={`Remove this PC of ${m.name}?`}
                description={
                  k.is_me
                    ? `This is this PC. You'll lose access to ${props.account} here.`
                    : m.keys.length === 1
                      ? "This is their last PC; they'll be removed."
                      : REMOVE_PC_TEXT
                }
                confirmLabel="Remove this PC"
                ariaLabel={`Remove this PC ${k.fingerprint}`}
                onConfirm={() =>
                  props.onEdit({
                    kind: "remove_key",
                    member_id: m.id,
                    key: k.key,
                  })
                }
              >
                Remove this PC
              </ConfirmButton>
            )}
          </li>
        ))}
      </ul>
      {picking && (
        <MemberWorkspacesDialog
          member={m}
          orgId={props.orgId}
          onSave={(workspaces) => {
            props.onEdit({
              kind: "set_workspaces",
              member_id: m.id,
              workspaces,
            });
            setPicking(false);
          }}
          onClose={() => setPicking(false)}
        />
      )}
      <AlertDialog open={!!ownRole} onOpenChange={(open) => !open && setOwnRole(undefined)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Change your own role to {ownRole ? roleLabel(ownRole) : ""}?
            </AlertDialogTitle>
            <AlertDialogDescription>{OWN_ROLE}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={onSingleClick(() => {
                if (ownRole)
                  props.onEdit({
                    kind: "set_role",
                    member_id: m.id,
                    role: ownRole,
                  });
              })}
            >
              Change role
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </li>
  );
}

/// A list signed on this PC but not published (accounts spec §6.5): what it changes and why it
/// waits, Retry and Discard, and for a SOPS file, which Dots can't write, the list to copy.
export function DraftBanner(props: {
  view: MembersView;
  blocker: string | undefined;
  discardBlocker: string | undefined;
  busy: boolean;
  retry: () => void;
  discard: () => void;
}) {
  const draft = props.view.draft;
  if (!draft) return null;
  return (
    <section
      aria-label="Unpublished member list"
      className="flex flex-col gap-1.5 rounded-md border border-needs-border bg-needs-surface p-2 text-xs"
    >
      <p className="font-medium">
        Not published yet: {draft.summary} (list #{draft.seq})
      </p>
      {draft.error && <p className="text-needs">{draft.error}</p>}
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={props.busy || !!props.blocker}
          title={props.blocker ?? (props.busy ? BUSY : undefined)}
          onClick={props.retry}
        >
          Retry
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={props.busy || !!props.discardBlocker}
          title={props.discardBlocker ?? (props.busy ? BUSY : undefined)}
          onClick={props.discard}
        >
          Discard
        </Button>
        {!props.view.publishes && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() =>
              navigator.clipboard
                .writeText(draft.signed)
                .then(() => toast.success("Copied the signed list"), notifyError)
            }
          >
            <Copy aria-hidden /> Copy signed list
          </Button>
        )}
      </div>
    </section>
  );
}

/// The workspaces of an account a member is in (workspaces spec): a checkbox and a role for
/// each. Default is always on, so it can't be unchecked. Saving sends the whole set.
function MemberWorkspacesDialog(props: {
  member: MemberView;
  orgId: string;
  onSave: (workspaces: Record<string, WorkspaceRole>) => void;
  onClose: () => void;
}) {
  const { member: m } = props;
  const list = useQuery({
    queryKey: keys.workspaces(props.orgId),
    queryFn: () => commands.workspaceList(props.orgId),
  });
  const [picked, setPicked] = useState<Record<string, WorkspaceRole>>({
    default: "member",
    ...m.workspaces,
  });
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Workspaces of {m.name}</DialogTitle>
          <DialogDescription>
            The workspaces they are in and their role in each. Default is always on.
          </DialogDescription>
        </DialogHeader>
        {list.isError && <ErrorNote error={list.error} />}
        <ul aria-label="Workspaces" className="flex flex-col gap-2">
          {(list.data ?? []).map((w) => {
            const role = picked[w.id];
            const isDefault = w.id === "default";
            return (
              <li key={w.id} className="flex items-center gap-2 text-sm">
                <label className="flex flex-1 items-center gap-2">
                  <input
                    type="checkbox"
                    checked={isDefault || !!role}
                    disabled={isDefault}
                    title={isDefault ? "Default is always on." : undefined}
                    onChange={(e) =>
                      setPicked((p) => {
                        const next = { ...p };
                        if (e.target.checked) next[w.id] = "member";
                        else delete next[w.id];
                        return next;
                      })
                    }
                  />
                  {w.name}
                </label>
                <NativeSelect
                  size="sm"
                  aria-label={`Role in ${w.name}`}
                  value={role ?? "member"}
                  disabled={!role}
                  title={role ? undefined : "Add them to the workspace first."}
                  onChange={(e) =>
                    setPicked((p) => ({
                      ...p,
                      [w.id]: e.target.value as WorkspaceRole,
                    }))
                  }
                >
                  {WORKSPACE_ROLES.map((r) => (
                    <NativeSelectOption key={r} value={r}>
                      {roleLabel(r)}
                    </NativeSelectOption>
                  ))}
                </NativeSelect>
              </li>
            );
          })}
        </ul>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!list.data}
            title={list.data ? undefined : "Loading the workspaces."}
            onClick={() => props.onSave(picked)}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
