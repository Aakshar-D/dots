import { useState } from "react";

import { commands } from "@/api/commands";
import type { RequestPreview, Role, RosterEdit } from "@/api/types";
import { Field } from "@/components/Field";
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
import { Textarea } from "@/components/ui/textarea";
import { roleLabel } from "@/lib/access";
import { errorMessage } from "@/lib/notify";

import { INVITE_ROLES } from "./InviteDialog";

/// Approve a request code someone sent (accounts spec §6.4): the code, what it asks, then
/// Approve. A request with a `problem` can't be approved; one with no invite from this PC asks
/// to be confirmed, with a role (unless it adds a PC to a member already on the list, who keeps
/// theirs). `approve` makes the edit; its error shows here.
export function ApproveDialog(props: {
  orgId: string;
  account: string;
  approve: (edit: RosterEdit) => Promise<unknown>;
  onClose: () => void;
}) {
  const [code, setCode] = useState("");
  const [preview, setPreview] = useState<RequestPreview>();
  const [confirmed, setConfirmed] = useState(false);
  const [role, setRole] = useState<Role>("member");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  async function check() {
    setBusy(true);
    setError(undefined);
    try {
      setPreview(await commands.requestInspect(props.orgId, code.trim()));
      setConfirmed(false);
    } catch (e) {
      setError(errorMessage(e));
    }
    setBusy(false);
  }

  const noInvite = !!preview && preview.invite === null;
  // A request that joins a member keeps their role: no role is picked.
  const picksRole = noInvite && !preview?.joins_member;
  async function approve() {
    if (!preview) return;
    setBusy(true);
    setError(undefined);
    try {
      await props.approve({
        kind: "approve",
        request: code.trim(),
        role: picksRole ? role : null,
        confirm_no_invite: noInvite && confirmed,
      });
      props.onClose();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  }

  const blocked = !preview
    ? "Check the request code first."
    : preview.problem
      ? preview.problem
      : noInvite && !confirmed
        ? "Confirm that this person has no invite from this PC first."
        : undefined;

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Approve a request to {props.account}</DialogTitle>
          <DialogDescription>
            Paste the request code they sent you. Approving signs a new member list with this PC's
            Dots ID.
          </DialogDescription>
        </DialogHeader>
        <Field label="Request code" htmlFor="request-code">
          <Textarea
            id="request-code"
            rows={3}
            className="font-mono text-xs"
            value={code}
            onChange={(e) => {
              setCode(e.target.value);
              setPreview(undefined);
            }}
          />
        </Field>
        {preview && (
          <>
            <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
              <dt className="text-muted-foreground">Name</dt>
              <dd>{preview.name}</dd>
              <dt className="text-muted-foreground">Email</dt>
              <dd>{preview.email}</dd>
              <dt className="text-muted-foreground">Dots ID fingerprint</dt>
              <dd className="font-mono">{preview.fingerprint}</dd>
              {preview.role && (
                <>
                  <dt className="text-muted-foreground">Role</dt>
                  <dd>{roleLabel(preview.role)}</dd>
                </>
              )}
              {preview.age_recipient && (
                <>
                  <dt className="text-muted-foreground">Age key for the SOPS file</dt>
                  <dd className="font-mono break-all">{preview.age_recipient}</dd>
                </>
              )}
            </dl>
            {preview.joins_member && (
              <p className="text-xs text-muted-foreground">
                Approving adds this PC to {preview.joins_member.name}, who stays{" "}
                {roleLabel(preview.joins_member.role)}.
              </p>
            )}
            {preview.problem ? (
              <p role="alert" className="text-sm text-destructive">
                {preview.problem}
              </p>
            ) : (
              noInvite && (
                <>
                  <label className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={confirmed}
                      onChange={(e) => setConfirmed(e.target.checked)}
                    />
                    This person has no invite from this PC
                  </label>
                  {picksRole && (
                    <Field label="Role" htmlFor="approve-role">
                      <NativeSelect
                        id="approve-role"
                        value={role}
                        onChange={(e) => setRole(e.target.value as Role)}
                      >
                        {INVITE_ROLES.map((r) => (
                          <NativeSelectOption key={r} value={r}>
                            {roleLabel(r)}
                          </NativeSelectOption>
                        ))}
                      </NativeSelect>
                    </Field>
                  )}
                </>
              )
            )}
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          {!preview ? (
            <Button
              type="button"
              disabled={busy || code.trim() === ""}
              title={code.trim() === "" ? "Paste the request code first." : undefined}
              onClick={() => void check()}
            >
              Check
            </Button>
          ) : (
            <Button
              type="button"
              disabled={busy || !!blocked}
              title={blocked}
              onClick={() => void approve()}
            >
              Approve
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
