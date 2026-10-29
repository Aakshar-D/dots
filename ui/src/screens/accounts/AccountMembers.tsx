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
  RosterEdit,
} from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { accessOf, notAllowedText, roleLabel } from "@/lib/access";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ApproveDialog } from "./ApproveDialog";
import { INVITE_ROLES, InviteDialog } from "./InviteDialog";

/// The core's refusal for an edit that touches an Owner (`accounts::edit::OWNER_LOCKED`).
export const OWNER_TEXT =
  "Owners can only be changed with the account owner's key, using dots-admin.";

/// An Admin whose Dots ID isn't in the policy's `admins` (members spec §6.1).
const NOT_SIGNING = "Can sign the list once the owner adds their Dots ID to the policy's admins.";

/// Why a control is off for the moment while the last change is sent.
const BUSY = "Waiting for the last change to finish.";

const REMOVE_TEXT =
  "Also revoke their access in the source: the list stops Dots honouring them, only the " +
  "source cuts them off.";

/// The Members tab of an account (accounts spec §6.5, §7; members spec §6.2): who is on the
/// list and with which PCs, its open invites, and, for an Admin or Owner whose Dots ID signs,
/// Invite, Approve request, role changes and removals. Every edit is made against the list's
/// `seq`; a list that changed meanwhile is refused with the core's words and asked for again.
export function AccountMembers({ org }: { org: OrgSummary }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.orgMembers(org.id),
    queryFn: () => commands.memberList(org.id),
  });
  // The core's words for the last edit that failed.
  const [notice, setNotice] = useState<string>();
  const [dialog, setDialog] = useState<"invite" | "approve">();

  // What changed the list asks for it and for the org's row again (both are under `orgs`). A
  // refusal does too: the list may have changed (a `conflict`) or its draft been dropped.
  const changed = () => invalidate(qc, keys.orgs);
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

  if (list.isError) return <ErrorNote error={list.error} />;
  const view = list.data;
  if (!view) return <p className="text-xs text-muted-foreground">Loading the member list…</p>;

  const edit = (e: RosterEdit) => () => commands.memberEdit(org.id, view.seq, e);
  // Why the list can't be changed from this PC, or undefined when it can.
  const manageBlocker = view.can_manage
    ? undefined
    : notAllowedText(org.name, accessOf(org, org.id), "members");
  const blocker =
    manageBlocker ??
    (view.can_sign
      ? undefined
      : (view.sign_blocker ?? `${org.name}: this PC's Dots ID can't sign the member list.`));
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
          disabled={!!blocker}
          title={blocker}
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
            lock={m.owner_locked ? OWNER_TEXT : blocker}
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
/// controls `lock` (why they are off) allows.
function MemberRow(props: {
  member: MemberView;
  lock: string | undefined;
  busy: boolean;
  onEdit: (edit: RosterEdit) => void;
}) {
  const { member: m, lock } = props;
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
          onChange={(e) =>
            props.onEdit({
              kind: "set_role",
              member_id: m.id,
              role: e.target.value as Role,
            })
          }
        >
          {roles.map((r) => (
            <NativeSelectOption key={r} value={r} disabled={r === "owner"}>
              {roleLabel(r)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
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
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={off}
              title={why}
              aria-label={`Remove this PC ${k.fingerprint}`}
              onClick={() =>
                props.onEdit({
                  kind: "remove_key",
                  member_id: m.id,
                  key: k.key,
                })
              }
            >
              Remove this PC
            </Button>
          </li>
        ))}
      </ul>
    </li>
  );
}

/// A list signed on this PC but not published (accounts spec §6.5): what it changes and why it
/// waits, Retry and Discard, and for a SOPS file, which Dots can't write, the list to copy.
function DraftBanner(props: {
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
