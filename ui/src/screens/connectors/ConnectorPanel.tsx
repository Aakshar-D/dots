import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ExternalLink } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ConnectorEntry } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Field } from "@/components/Field";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";

import { cleaned, fieldError, isConnected, makerLabel, STATE } from "./fields";

function Section(props: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-2" aria-label={props.title}>
      <h3 className="text-xs font-semibold uppercase text-muted-foreground">{props.title}</h3>
      {props.children}
    </section>
  );
}

/// The Personal dots, as checkboxes: a connector's values are Personal, so an org's dot cannot
/// use it.
function DotPicker(props: {
  name: string;
  picked: string[];
  onChange: (ids: string[]) => void;
}) {
  const dots = useQuery({ queryKey: keys.dots, queryFn: commands.listDots });
  const personal = (dots.data ?? []).filter((d) => d.org_id === null);
  if (dots.isPending) return null;
  if (personal.length === 0) {
    return (
      <p className="text-xs text-muted-foreground">
        You have no sols yet. Connect now and pick sols later.
      </p>
    );
  }
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="sr-only">{`Sols that may use ${props.name}`}</legend>
      {personal.map((dot) => {
        const checked = props.picked.includes(dot.id);
        return (
          <label key={dot.id} className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              className="size-4 accent-primary"
              checked={checked}
              onChange={() =>
                props.onChange(
                  checked ? props.picked.filter((id) => id !== dot.id) : [...props.picked, dot.id],
                )
              }
            />
            {dot.name}
          </label>
        );
      })}
    </fieldset>
  );
}

/// The steps for getting a key (or what signing in does), and the button that opens the page.
function Steps({ connector }: { connector: ConnectorEntry }) {
  const open = useMutation({
    mutationFn: () => commands.connectorOpenPage(connector.id, "instructions"),
    onError: notifyError,
  });
  const title =
    connector.auth === "key"
      ? "How to get your key"
      : connector.auth === "oauth"
        ? "How signing in works"
        : "Why it is not available";
  return (
    <Section title={title}>
      <ol className="flex list-decimal flex-col gap-1.5 pl-5 text-sm">
        {connector.instructions.steps.map((step) => (
          <li key={step}>{step}</li>
        ))}
      </ol>
      <div>
        <Button
          size="sm"
          variant={connector.auth === "key" ? "default" : "outline"}
          onClick={() => open.mutate()}
        >
          <ExternalLink aria-hidden /> {connector.instructions.open_label}
        </Button>
      </div>
    </Section>
  );
}

/// Whether this PC has Node.js for a stdio connector, and where to get it when it does not.
function RuntimeNote({ connector }: { connector: ConnectorEntry }) {
  const need = connector.runtime;
  const check = useQuery({
    queryKey: keys.connectorRuntime(connector.id),
    queryFn: () => commands.connectorRuntime(connector.id),
    enabled: need !== null,
  });
  const open = useMutation({
    mutationFn: () => commands.connectorOpenPage(connector.id, "runtime"),
    onError: notifyError,
  });
  if (!need || !check.data || check.data.ok) return null;
  return (
    <Alert>
      <AlertDescription className="flex flex-col gap-2">
        <span>
          {`${connector.name} needs ${need.name} ${need.min_major} or newer on this PC. `}
          {check.data.found
            ? `This PC has ${check.data.found}.`
            : `${need.name} was not found.`}{" "}
          Install it, then open this panel again.
        </span>
        <span>
          <Button size="sm" variant="outline" onClick={() => open.mutate()}>
            <ExternalLink aria-hidden /> {`Get ${need.name}`}
          </Button>
        </span>
      </AlertDescription>
    </Alert>
  );
}

/// The panel of one connector: what it does and who makes it, how to get its key or sign in,
/// the dots that may use it, and Connect, Test, Save dots or Disconnect.
export function ConnectorPanel(props: { connector: ConnectorEntry; onClose: () => void }) {
  const { connector } = props;
  const qc = useQueryClient();
  const connected = isConnected(connector);
  const [picked, setPicked] = useState<string[]>(connector.used_by.map((d) => d.id));
  const [values, setValues] = useState<Record<string, string>>({});
  const [touched, setTouched] = useState(false);
  const runtime = useQuery({
    queryKey: keys.connectorRuntime(connector.id),
    queryFn: () => commands.connectorRuntime(connector.id),
    enabled: connector.runtime !== null,
  });
  const runtimeOk = connector.runtime === null || runtime.data?.ok === true;

  const done = () => {
    invalidate(qc, keys.library);
    invalidateDots(qc);
  };
  const connect = useMutation({
    mutationFn: () => {
      const clean = Object.fromEntries(
        connector.fields.map((f) => [f.key, cleaned(f, values[f.key] ?? "")]),
      );
      return commands.connectorConnect(connector.id, clean, picked);
    },
    onSuccess: () => {
      setValues({});
      setTouched(false);
      done();
    },
    onError: notifyError,
  });
  const signIn = useMutation({
    mutationFn: () => commands.connectorSignIn(connector.id, picked),
    onSuccess: done,
    onError: notifyError,
  });
  const saveDots = useMutation({
    mutationFn: () => commands.connectorSetDots(connector.id, picked),
    onSuccess: done,
    onError: notifyError,
  });
  const test = useMutation({
    mutationFn: () => commands.libraryCheck(connector.server_id ?? ""),
    onSuccess: done,
    onError: notifyError,
  });
  const disconnect = useMutation({
    mutationFn: () => commands.connectorDisconnect(connector.id),
    onSuccess: () => {
      done();
      props.onClose();
    },
    onError: notifyError,
  });

  const errors = Object.fromEntries(
    connector.fields.map((f) => [f.key, fieldError(f, values[f.key] ?? "")]),
  );
  const valid = Object.values(errors).every((e) => e === null);
  const dotsChanged =
    [...picked].sort().join() !== connector.used_by.map((d) => d.id).sort().join();
  const busy =
    connect.isPending || signIn.isPending || saveDots.isPending || disconnect.isPending;
  const state = STATE[connector.state];

  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex flex-wrap items-center gap-2">
            {connector.name}
            <Badge variant="secondary" className={state.className}>
              {state.label}
            </Badge>
          </DialogTitle>
          <DialogDescription>{connector.description}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{`Made ${makerLabel(connector)}`}</span>
            <Button
              size="sm"
              variant="link"
              className="h-auto p-0 text-xs"
              onClick={() => void commands.connectorOpenPage(connector.id, "source").catch(notifyError)}
            >
              {connector.trust === "community" ? "See its code" : "Learn more"}
            </Button>
          </div>

          {connector.note && (
            <Alert>
              <AlertDescription>{connector.note}</AlertDescription>
            </Alert>
          )}

          {connector.error && (
            <p role="status" className="text-sm text-destructive">
              {connector.error}
            </p>
          )}
          {connector.state === "name_taken" && (
            <Link to="/library" className="text-sm text-primary underline-offset-4 hover:underline">
              Open the Library
            </Link>
          )}

          {connector.abilities.length > 0 && (
            <Section title="What your sols can do">
              <ul className="flex list-disc flex-col gap-1 pl-5 text-sm">
                {connector.abilities.map((a) => (
                  <li key={a}>{a}</li>
                ))}
              </ul>
            </Section>
          )}

          {connected && (
            <Section title="Connection">
              <p className="text-sm">
                {connector.tools !== null
                  ? `${connector.tools} ${connector.tools === 1 ? "tool" : "tools"} available.`
                  : "Not checked yet."}{" "}
                {connector.used_by.length > 0
                  ? `Used by ${connector.used_by.map((d) => d.name).join(", ")}.`
                  : "No sol uses it yet."}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={test.isPending}
                  onClick={() => test.mutate()}
                >
                  {test.isPending ? "Testing…" : "Test"}
                </Button>
                <ConfirmButton
                  title={`Disconnect ${connector.name}?`}
                  description={
                    <>
                      {connector.used_by.length > 0
                        ? `${connector.used_by.map((d) => d.name).join(", ")} will stop using it. `
                        : ""}
                      Sage forgets its {connector.auth === "oauth" ? "sign-in" : "key"}. You can
                      also remove Sage's access on {connector.name}'s own site.
                    </>
                  }
                  confirmLabel="Disconnect"
                  disabled={busy}
                  onConfirm={() => disconnect.mutate()}
                >
                  Disconnect
                </ConfirmButton>
              </div>
            </Section>
          )}

          {connector.state !== "coming_soon" && connector.state !== "name_taken" && (
            <>
              <Steps connector={connector} />
              <RuntimeNote connector={connector} />

              {connector.auth === "key" && (
                <Section title={connected ? "Replace the key" : "Your key"}>
                  {connector.fields.map((f) => {
                    const id = `connector-${connector.id}-${f.key}`;
                    return (
                      <Field
                        key={f.key}
                        label={f.label}
                        htmlFor={id}
                        hint={f.hint}
                        error={touched && values[f.key] ? (errors[f.key] ?? undefined) : undefined}
                      >
                        <Input
                          id={id}
                          type={f.secret ? "password" : "text"}
                          autoComplete="off"
                          spellCheck={false}
                          placeholder={f.placeholder}
                          value={values[f.key] ?? ""}
                          onChange={(e) => {
                            setTouched(true);
                            setValues({ ...values, [f.key]: e.target.value });
                          }}
                        />
                      </Field>
                    );
                  })}
                </Section>
              )}

              <Section title="Sols that may use it">
                <DotPicker name={connector.name} picked={picked} onChange={setPicked} />
              </Section>

              <div className="flex flex-wrap items-center gap-2">
                {connector.auth === "key" && (
                  <Button
                    disabled={!valid || !runtimeOk || busy}
                    onClick={() => connect.mutate()}
                  >
                    {connect.isPending ? "Connecting…" : connected ? "Save key" : "Connect"}
                  </Button>
                )}
                {connector.auth === "oauth" && !signIn.isPending && (
                  <Button disabled={busy} onClick={() => signIn.mutate()}>
                    {connected ? `Sign in to ${connector.name} again` : `Sign in with ${connector.name}`}
                  </Button>
                )}
                {signIn.isPending && (
                  <>
                    <span role="status" className="text-sm text-muted-foreground">
                      Finish signing in in your browser…
                    </span>
                    <Button
                      variant="outline"
                      onClick={() => void commands.connectorSignInCancel(connector.id).catch(notifyError)}
                    >
                      Cancel sign-in
                    </Button>
                  </>
                )}
                {connected && (
                  <Button
                    variant="outline"
                    disabled={!dotsChanged || busy}
                    onClick={() => saveDots.mutate()}
                  >
                    {saveDots.isPending ? "Saving…" : "Save sols"}
                  </Button>
                )}
              </div>
            </>
          )}

          {connector.state === "coming_soon" && <Steps connector={connector} />}
        </div>
      </DialogContent>
    </Dialog>
  );
}
