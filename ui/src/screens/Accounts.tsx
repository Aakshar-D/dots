import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { CommandError, commands } from "@/api/commands";
import type { JoinPreview, OrgSummary, SourceCreds } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
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
import { Textarea } from "@/components/ui/textarea";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

/// How an org's status reads: plain while it works; the attention colour while it works from
/// its saved copy, has refused a new policy, or waits for the admin; the error colour once its
/// values are gone. The Library's Env tab shows the status in the same tone.
export const ORG_TONE: Record<OrgSummary["status"], string> = {
  connected: "",
  offline: "text-needs",
  policy_invalid: "text-needs",
  pending: "text-needs",
  revoked: "text-destructive",
  expired: "text-destructive",
};

const SOURCE: Record<string, string> = {
  openbao: "OpenBao",
  infisical: "Infisical",
  sops: "SOPS file",
};

/// "OpenBao at https://…", as the core's lines of what an Update changes name a source.
export function sourceText(kind: string, address: string): string {
  return `${SOURCE[kind] ?? "An unreadable source"} at ${address}`;
}

const WAITING = "Waiting for your admin to add this PC's key";

/// One joined org: where its values come from, how it stands, and Refresh, Update and Remove.
function OrgRow({ org, onUpdate }: { org: OrgSummary; onUpdate: () => void }) {
  const qc = useQueryClient();
  const [policyShown, setPolicyShown] = useState(false);
  const refresh = useMutation({
    mutationFn: () => commands.orgRefresh(org.id),
    // A failed fetch is a status, not an error: the list shows it.
    onSuccess: () => invalidate(qc, keys.orgs),
    onError: notifyError,
  });
  const remove = useMutation({
    mutationFn: () => commands.orgRemove(org.id),
    onSuccess: () => {
      invalidate(qc, keys.orgs);
      toast.success(`Left ${org.name}`);
    },
    onError: notifyError,
  });
  const policyId = `org-policy-${org.id}`;
  return (
    <li aria-label={org.name} className="flex flex-col gap-2 rounded-md border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex min-w-0 flex-1 flex-col">
          <span className="font-medium">{org.name}</span>
          <p className={`text-xs first-letter:uppercase ${ORG_TONE[org.status]}`}>
            {org.status_text}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={refresh.isPending}
          onClick={() => refresh.mutate()}
        >
          {refresh.isPending ? "Refreshing…" : "Refresh"}
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={onUpdate}>
          Update
        </Button>
        <ConfirmButton
          title={`Remove ${org.name}?`}
          description={
            `The dots of ${org.name} stay on this PC, but they can't run until you join it ` +
            "again. The saved copy of its variables and your sign-in details for it are deleted."
          }
          confirmLabel="Remove"
          disabled={remove.isPending}
          onConfirm={() => remove.mutate()}
        >
          Remove
        </ConfirmButton>
      </div>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Source</dt>
        <dd className="break-all">{sourceText(org.source_kind, org.address)}</dd>
        <dt className="text-muted-foreground">Key fingerprint</dt>
        <dd className="font-mono">{org.key_fingerprint}</dd>
        <dt className="text-muted-foreground">Last fetch</dt>
        <dd>{localDateTime(org.last_fetch_at) || "Never"}</dd>
        <dt className="text-muted-foreground">Saved copy until</dt>
        <dd>{localDateTime(org.copy_until) || "No saved copy"}</dd>
      </dl>
      {org.policy_summary.length === 0 ? (
        <p className="text-xs text-muted-foreground">No policy accepted yet.</p>
      ) : (
        <>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="self-start"
            aria-expanded={policyShown}
            aria-controls={policyId}
            onClick={() => setPolicyShown(!policyShown)}
          >
            {policyShown ? "Hide the policy" : "Show the policy"}
          </Button>
          {policyShown && (
            <ul id={policyId} className="flex flex-col gap-0.5 pl-3 text-xs">
              {org.policy_summary.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )}
        </>
      )}
    </li>
  );
}

type CredsKind = "token" | "login" | "machine" | "age";

/// The sign-in details the code's source asks for: a token or a username and password for
/// OpenBao (as the code names), a machine identity for Infisical, this PC's key for SOPS.
function credsKind(preview: JoinPreview): CredsKind | null {
  switch (preview.source_kind) {
    case "openbao":
      return preview.auth === "userpass" || preview.auth === "ldap" ? "login" : "token";
    case "infisical":
      return "machine";
    case "sops":
      return "age";
    default:
      return null;
  }
}

type Fields = {
  token: string;
  username: string;
  password: string;
  clientId: string;
  clientSecret: string;
};
const BLANK: Fields = { token: "", username: "", password: "", clientId: "", clientSecret: "" };
/// The fields without what is secret: what is left after a submit.
const unsecret = (f: Fields): Fields => ({ ...f, token: "", password: "", clientSecret: "" });

/// The details as `SourceCreds`; null when none were typed (and always for SOPS, whose key the
/// core holds). "partial" when some are missing.
function credsOf(kind: CredsKind | null, f: Fields): SourceCreds | null | "partial" {
  const typed = (values: string[]) =>
    values.every((v) => v !== "") ? "all" : values.every((v) => v === "") ? "none" : "some";
  const check = (values: string[], creds: SourceCreds) => {
    const t = typed(values);
    return t === "all" ? creds : t === "none" ? null : "partial";
  };
  switch (kind) {
    case "token":
      return check([f.token], { kind: "token", token: f.token });
    case "login":
      return check([f.username.trim(), f.password], {
        kind: "login",
        username: f.username.trim(),
        password: f.password,
      });
    case "machine":
      return check([f.clientId.trim(), f.clientSecret], {
        kind: "machine",
        client_id: f.clientId.trim(),
        client_secret: f.clientSecret,
      });
    default:
      return null;
  }
}

/// This PC's public key for a SOPS org, to send to the admin. It is made on the first ask.
function SopsKey(props: { orgId: string; verb: string }) {
  const key = useQuery({
    queryKey: keys.orgSopsKey(props.orgId),
    queryFn: () => commands.orgSopsKey(props.orgId),
  });
  if (key.isError) return <ErrorNote error={key.error} />;
  return (
    <div className="flex flex-col gap-2">
      <Field
        label="This PC's key"
        htmlFor="org-sops-key"
        hint={`Send this key to your admin, then press ${props.verb}`}
      >
        <div className="flex gap-2">
          <Input id="org-sops-key" readOnly className="font-mono" value={key.data ?? ""} />
          <Button
            type="button"
            variant="outline"
            size="sm"
            aria-label="Copy the key"
            disabled={!key.data}
            onClick={() =>
              navigator.clipboard
                .writeText(key.data ?? "")
                .then(() => toast.success("Copied the key"), notifyError)
            }
          >
            <Copy aria-hidden /> Copy
          </Button>
        </div>
      </Field>
    </div>
  );
}

type Step = "code" | "preview" | "creds";

/// Join an org (no `org`), or Update `org` with a new code from its admin. Three steps: the
/// code, what it joins or changes, then the sign-in details its source needs. Secret fields
/// are cleared after every submit, and go with the dialog when it closes.
export function JoinDialog(props: { org?: OrgSummary; onClose: () => void }) {
  const qc = useQueryClient();
  const { org } = props;
  const [step, setStep] = useState<Step>("code");
  const [code, setCode] = useState("");
  const [preview, setPreview] = useState<JoinPreview>();
  const [fields, setFields] = useState(BLANK);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [declined, setDeclined] = useState(false);
  const [waiting, setWaiting] = useState(false);
  const set = (change: Partial<Fields>) => setFields({ ...fields, ...change });

  async function look(text: string) {
    setBusy(true);
    setError(undefined);
    try {
      setPreview(await commands.orgPreview(text));
      setStep("preview");
    } catch (e) {
      setError(errorMessage(e));
    }
    setBusy(false);
  }

  function readFile(file: File | undefined) {
    if (!file) return;
    file.text().then(
      (text) => {
        setCode(text);
        void look(text);
      },
      (e: unknown) => setError(errorMessage(e)),
    );
  }

  const kind = preview ? credsKind(preview) : null;
  const creds = credsOf(kind, fields);
  async function submit() {
    if (creds === "partial") return;
    setFields(unsecret(fields));
    setBusy(true);
    setError(undefined);
    setDeclined(false);
    try {
      const joined = org
        ? await commands.orgUpdate(org.id, code, creds)
        : await commands.orgJoin(code, creds);
      invalidate(qc, keys.orgs);
      if (joined.status === "pending") {
        setWaiting(true);
        setBusy(false);
        return;
      }
      toast.success(org ? `Updated ${joined.name}` : `Joined ${joined.name}`);
      props.onClose();
    } catch (e) {
      setBusy(false);
      // The person said no to the new policy key in the system dialog: nothing changed.
      if (e instanceof CommandError && e.kind === "cancelled") {
        setDeclined(true);
        setStep("preview");
      } else {
        setError(errorMessage(e));
      }
    }
  }

  function back() {
    setFields(unsecret(fields));
    setError(undefined);
    setWaiting(false);
    setStep(step === "creds" ? "preview" : "code");
  }

  // A code for an org joined already goes through its Update, and Update takes only its own.
  const refusal = !preview
    ? undefined
    : org && preview.org_id !== org.id
      ? `This join code is for org '${preview.org_id}', not '${org.id}'.`
      : !org && preview.existing
        ? `${preview.name} is joined already: use Update on it in the list.`
        : undefined;
  const verb = org ? "Update" : "Join";
  // An Update to a changed source never reuses the saved details: the core would refuse.
  const fresh = !org || !!preview?.source_changed;
  const required = fresh && creds === null && kind !== "age";

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{org ? `Update ${org.name}` : "Join an account"}</DialogTitle>
          <DialogDescription>
            {step === "code"
              ? "Paste the join code from your admin, or open the .dotsorg file they sent."
              : step === "preview"
                ? org
                  ? "Check what this code changes."
                  : "Check what this code joins."
                : kind === "age"
                  ? "This account's variables are in a file encrypted to each person's key."
                  : "Your sign-in details for the account's source. Dots keeps them in Windows " +
                    "Credential Manager and never shows them."}
          </DialogDescription>
        </DialogHeader>

        {step === "code" && (
          <>
            <Field
              label="Join code"
              htmlFor="org-code"
              hint={
                "The code says where the account's variables are. It holds no secret, unless its " +
                "SOPS file's URL carries a token: then keep it as private as a password."
              }
            >
              <Textarea
                id="org-code"
                rows={4}
                className="font-mono text-xs"
                value={code}
                onChange={(e) => setCode(e.target.value)}
              />
            </Field>
            <Field label="Or open a .dotsorg file" htmlFor="org-code-file">
              <Input
                id="org-code-file"
                type="file"
                accept=".dotsorg"
                onChange={(e) => readFile(e.target.files?.[0])}
              />
            </Field>
          </>
        )}

        {step !== "code" && preview && (
          <>
            {step === "preview" && (
              <>
                <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-sm">
                  <dt className="text-muted-foreground">Account</dt>
                  <dd>
                    {preview.name}{" "}
                    <span className="font-mono text-xs text-muted-foreground">
                      {preview.org_id}
                    </span>
                  </dd>
                  <dt className="text-muted-foreground">Source</dt>
                  <dd className="break-all">
                    {sourceText(preview.source_kind, preview.address)}
                  </dd>
                  <dt className="text-muted-foreground">Key fingerprint</dt>
                  <dd className="font-mono">{preview.key_fingerprint}</dd>
                </dl>
                {refusal ? (
                  <p role="alert" className="text-sm text-destructive">
                    {refusal}
                  </p>
                ) : org ? (
                  preview.changes.length === 0 ? (
                    <p className="text-sm text-muted-foreground">This code changes nothing.</p>
                  ) : (
                    <ul aria-label="What changes" className="flex flex-col gap-1 text-sm">
                      {/* The core puts the line that warns of a new key last. */}
                      {preview.changes.map((line, i) =>
                        preview.key_changed && i === preview.changes.length - 1 ? (
                          <li key={line} className="flex gap-1.5 font-medium text-needs">
                            <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
                            {line}
                          </li>
                        ) : (
                          <li key={line}>{line}</li>
                        ),
                      )}
                    </ul>
                  )
                ) : (
                  <p className="text-xs text-muted-foreground">
                    Check the key fingerprint with your admin before you go on.
                  </p>
                )}
                {declined && (
                  <p role="status" className="text-sm text-needs">
                    Not updated: you declined the new policy key.
                  </p>
                )}
              </>
            )}

            {step === "creds" && (
              <>
                {org && kind !== "age" && !fresh && (
                  <p className="text-xs text-muted-foreground">
                    Leave these empty to keep the saved sign-in details.
                  </p>
                )}
                {org && kind !== "age" && fresh && (
                  <p className="text-xs text-needs">
                    The source changed, so enter your sign-in details again: Dots does not send
                    the saved ones to it.
                  </p>
                )}
                {kind === "token" && (
                  <Field label="Token" htmlFor="org-token">
                    <Input
                      id="org-token"
                      type="password"
                      autoComplete="off"
                      className="font-mono"
                      value={fields.token}
                      onChange={(e) => set({ token: e.target.value })}
                    />
                  </Field>
                )}
                {kind === "login" && (
                  <>
                    <Field label="Username" htmlFor="org-username">
                      <Input
                        id="org-username"
                        autoComplete="off"
                        value={fields.username}
                        onChange={(e) => set({ username: e.target.value })}
                      />
                    </Field>
                    <Field
                      label="Password"
                      htmlFor="org-password"
                      hint={preview.auth === "ldap" ? "Your directory (LDAP) password." : undefined}
                    >
                      <Input
                        id="org-password"
                        type="password"
                        autoComplete="off"
                        value={fields.password}
                        onChange={(e) => set({ password: e.target.value })}
                      />
                    </Field>
                  </>
                )}
                {kind === "machine" && (
                  <>
                    <p className="text-xs text-muted-foreground">
                      The machine identity your admin made for you.
                    </p>
                    <Field label="Client id" htmlFor="org-client-id">
                      <Input
                        id="org-client-id"
                        autoComplete="off"
                        className="font-mono"
                        value={fields.clientId}
                        onChange={(e) => set({ clientId: e.target.value })}
                      />
                    </Field>
                    <Field label="Client secret" htmlFor="org-client-secret">
                      <Input
                        id="org-client-secret"
                        type="password"
                        autoComplete="off"
                        className="font-mono"
                        value={fields.clientSecret}
                        onChange={(e) => set({ clientSecret: e.target.value })}
                      />
                    </Field>
                  </>
                )}
                {kind === "age" && <SopsKey orgId={preview.org_id} verb={verb} />}
                {creds === "partial" && (
                  <p className="text-xs text-muted-foreground">
                    {org && !fresh
                      ? "Give every field, or leave them all empty."
                      : "Give every field."}
                  </p>
                )}
                {waiting && (
                  <p role="status" className="text-sm text-needs">
                    {WAITING}
                  </p>
                )}
              </>
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
          {step !== "code" && (
            <Button type="button" variant="outline" disabled={busy} onClick={back}>
              Back
            </Button>
          )}
          {step === "code" && (
            <Button
              type="button"
              disabled={busy || code.trim() === ""}
              onClick={() => void look(code)}
            >
              Preview
            </Button>
          )}
          {step === "preview" && (
            <Button
              type="button"
              disabled={!!refusal}
              onClick={() => {
                setDeclined(false);
                setStep("creds");
              }}
            >
              Continue
            </Button>
          )}
          {step === "creds" && (
            <Button
              type="button"
              disabled={busy || creds === "partial" || required}
              onClick={() => void submit()}
            >
              {waiting ? "Try again" : verb}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// Settings → Accounts (env vault spec §9): the orgs joined on this PC, Join, and per org
/// Refresh, Update and Remove. Nothing here ever shows a value or sign-in details.
export function Accounts() {
  const list = useQuery({ queryKey: keys.orgs, queryFn: commands.orgList });
  // The org to update; null to join a new one; undefined while the dialog is closed.
  const [joining, setJoining] = useState<OrgSummary | null>();
  const orgs = list.data ?? [];
  return (
    <fieldset
      aria-label="Accounts"
      className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4"
    >
      <legend className="px-1 text-sm font-semibold">Accounts</legend>
      <p className="text-xs text-muted-foreground">
        An account's admin keeps its variables in OpenBao, Infisical or a SOPS file, and
        its signed policy says how dots may use them. Join one with the code your admin gives
        you.
      </p>
      {list.isError && <ErrorNote error={list.error} />}
      {list.isSuccess && orgs.length === 0 && (
        <p className="text-sm text-muted-foreground">No account is joined on this PC.</p>
      )}
      {orgs.length > 0 && (
        <ul className="flex flex-col gap-2">
          {orgs.map((org) => (
            <OrgRow key={org.id} org={org} onUpdate={() => setJoining(org)} />
          ))}
        </ul>
      )}
      <Button
        type="button"
        size="sm"
        variant="outline"
        className="self-start"
        onClick={() => setJoining(null)}
      >
        Join an account
      </Button>
      {joining !== undefined && (
        <JoinDialog org={joining ?? undefined} onClose={() => setJoining(undefined)} />
      )}
    </fieldset>
  );
}
