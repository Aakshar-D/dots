import { useQueryClient } from "@tanstack/react-query";
import { Copy, Download } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { CreatedInvite, Role } from "@/api/types";
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
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { roleLabel } from "@/lib/access";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// The roles an invite may offer: an Owner is only made with the owner's key (accounts spec §7).
export const INVITE_ROLES: Role[] = ["admin", "member", "viewer"];

/// Invite one person to `org` (accounts spec §7): their email and role, then the join code
/// that carries the offer, to copy or save as a .dotsorg file and send them.
export function InviteDialog(props: { orgId: string; account: string; onClose: () => void }) {
  const qc = useQueryClient();
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [made, setMade] = useState<CreatedInvite>();

  // The code as a file to save: the app has no save dialog, so the webview downloads it.
  const file = made ? `data:text/plain;charset=utf-8,${encodeURIComponent(made.code)}` : undefined;

  async function create() {
    setBusy(true);
    setError(undefined);
    try {
      setMade(await commands.inviteCreate(props.orgId, email.trim(), role));
      invalidate(qc, keys.orgs);
    } catch (e) {
      setError(errorMessage(e));
    }
    setBusy(false);
  }

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Invite to {props.account}</DialogTitle>
          <DialogDescription>
            {made
              ? "Send this code to them. It holds their invite, good until " +
                `${localDateTime(made.invite.expires_at)}.`
              : "They join with the code Dots makes, then send you their request to approve."}
          </DialogDescription>
        </DialogHeader>
        {made ? (
          <div className="flex flex-col gap-2">
            <p className="text-sm">
              {made.invite.email} · {roleLabel(made.invite.role)}
            </p>
            <Textarea
              readOnly
              aria-label="Invite code"
              rows={4}
              className="font-mono text-xs"
              value={made.code}
            />
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() =>
                  navigator.clipboard
                    .writeText(made.code)
                    .then(() => toast.success("Copied the invite code"), notifyError)
                }
              >
                <Copy aria-hidden /> Copy
              </Button>
              <Button asChild variant="outline" size="sm">
                <a href={file} download={`${props.orgId}.dotsorg`}>
                  <Download aria-hidden /> Save as .dotsorg
                </a>
              </Button>
            </div>
          </div>
        ) : (
          <>
            <Field label="Email" htmlFor="invite-email">
              <Input
                id="invite-email"
                type="email"
                autoComplete="off"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </Field>
            <Field label="Role" htmlFor="invite-role">
              <NativeSelect
                id="invite-role"
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
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            {made ? "Close" : "Cancel"}
          </Button>
          {!made && (
            <Button
              type="button"
              disabled={busy || email.trim() === ""}
              title={email.trim() === "" ? "Give their email first." : undefined}
              onClick={() => void create()}
            >
              Create invite
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
