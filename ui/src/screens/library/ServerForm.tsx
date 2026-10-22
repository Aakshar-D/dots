import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { McpServerConfig } from "@/api/types";
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
import { NativeSelect } from "@/components/ui/native-select";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, ToolList, type ServerEntry } from "./parts";
import { joinLines, movesToVault, parseLines, variablesIn } from "./text";
import { parseTools } from "./tools";

type Draft = {
  name: string;
  transport: "stdio" | "http";
  command: string;
  args: string;
  cwd: string;
  env: string;
  url: string;
  headers: string;
};

type Errors = Partial<
  Record<"name" | "command" | "args" | "cwd" | "env" | "url" | "headers", string>
>;

const NAME_ERROR =
  'Use 1 to 40 of a-z, 0-9, _ and -, with no "__" and no "_" at the start or end. "dots" is reserved.';
const CONTROL_ERROR = "Remove the control or direction characters.";
const URL_ERROR = "Enter a URL that starts with http:// or https:// and names a host.";

/// C0, DEL, C1 and the bidi marks the core refuses in a command, argument, folder or env key.
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u202a-\u202e\u2066-\u2069]/;
const hasControl = (text: string) => CONTROL.test(text);

function urlOk(url: string): boolean {
  if (hasControl(url) || url.includes("\\")) return false;
  const m = /^https?:\/\/([^/?#\s]*)/.exec(url);
  if (m === null) return false;
  const host = m[1]!.split("@").pop()!;
  return host !== "" && !host.startsWith(":");
}

function draftOf(entry: ServerEntry | null): Draft {
  const blank: Draft = {
    name: "",
    transport: "stdio",
    command: "",
    args: "",
    cwd: "",
    env: "",
    url: "",
    headers: "",
  };
  if (!entry) return blank;
  const config = entry.config;
  return config.transport === "stdio"
    ? {
        ...blank,
        name: entry.name,
        command: config.command,
        args: config.args.join("\n"),
        cwd: config.cwd ?? "",
        env: joinLines(config.env, "="),
      }
    : {
        ...blank,
        name: entry.name,
        transport: "http",
        url: config.url,
        headers: joinLines(config.headers, ":"),
      };
}

/// The config the form describes, or what is wrong with it. `name` is checked only when asked:
/// a test of the connection does not need one.
export function readDraft(
  draft: Draft,
  checkName: boolean,
): { config: McpServerConfig; errors: Errors } | { config: null; errors: Errors } {
  const errors: Errors = {};
  if (
    checkName &&
    (draft.name === "dots" ||
      !/^[a-z0-9_-]{1,40}$/.test(draft.name) ||
      draft.name.includes("__") ||
      draft.name.startsWith("_") ||
      draft.name.endsWith("_"))
  ) {
    errors.name = NAME_ERROR;
  }
  if (draft.transport === "stdio") {
    if (draft.command.trim() === "") errors.command = "Enter the command to run.";
    else if (hasControl(draft.command)) errors.command = CONTROL_ERROR;
    if (hasControl(draft.args.replace(/\r?\n/g, ""))) errors.args = CONTROL_ERROR;
    if (hasControl(draft.cwd)) errors.cwd = CONTROL_ERROR;
    const env = parseLines(draft.env, "=");
    if (!env.ok) errors.env = `Line ${env.line} needs the form KEY=VALUE.`;
    else if (Object.keys(env.entries).some(hasControl)) errors.env = CONTROL_ERROR;
    if (Object.keys(errors).length > 0 || !env.ok) return { config: null, errors };
    return {
      errors,
      config: {
        transport: "stdio",
        command: draft.command.trim(),
        args: draft.args.split(/\r?\n/).filter((a) => a.trim() !== ""),
        cwd: draft.cwd.trim() === "" ? null : draft.cwd.trim(),
        env: env.entries,
      },
    };
  }
  if (!urlOk(draft.url.trim())) errors.url = URL_ERROR;
  const headers = parseLines(draft.headers, ":");
  if (!headers.ok) errors.headers = `Line ${headers.line} needs the form Name: value.`;
  if (Object.keys(errors).length > 0 || !headers.ok) return { config: null, errors };
  return { errors, config: { transport: "http", url: draft.url.trim(), headers: headers.entries } };
}

/// The variables the draft reads with `${VAR}`.
function variablesOf(draft: Draft): string[] {
  return variablesIn(
    draft.transport === "stdio"
      ? [draft.command, draft.args, draft.cwd, draft.env]
      : [draft.url, draft.headers],
  );
}

/// The keys of env or headers that look like secrets and hold a plain value.
function secretKeys(draft: Draft): string[] {
  const parsed =
    draft.transport === "stdio" ? parseLines(draft.env, "=") : parseLines(draft.headers, ":");
  if (!parsed.ok) return [];
  return Object.entries(parsed.entries)
    .filter(([key, value]) => movesToVault(key, value))
    .map(([key]) => key);
}

/// The form to add an MCP server (`entry` is null) or to change one. Adding tests the
/// connection first when the person asks; saving then accepts the list that was shown.
export function ServerForm(props: { entry: ServerEntry | null; onClose: () => void }) {
  const { entry } = props;
  const qc = useQueryClient();
  const [draft, setDraft] = useState(() => draftOf(entry));
  const [tried, setTried] = useState(false);
  const set = (change: Partial<Draft>) => {
    setDraft({ ...draft, ...change });
    // The list that was shown belongs to the config that was tested.
    probe.reset();
  };

  const probe = useMutation({ mutationFn: commands.libraryProbe });
  const save = useMutation({
    mutationFn: (v: { config: McpServerConfig; accept: string | null }) => {
      const config = { kind: "mcp_server" as const, config: v.config };
      return entry
        ? commands.libraryUpdate(entry.id, config)
        : commands.libraryAdd(draft.name, config, v.accept);
    },
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
  });

  const variables = variablesOf(draft);
  const env = useQuery({
    queryKey: keys.environmentVariables(variables),
    queryFn: () => commands.environmentVariables(variables),
    enabled: variables.length > 0,
  });
  const notSet = (env.data ?? []).filter((v) => !v.set).map((v) => v.name);

  const { config, errors } = readDraft(draft, !entry);
  const shown = tried ? errors : {};
  function test() {
    setTried(true);
    const tested = readDraft(draft, false);
    if (tested.config) probe.mutate(tested.config);
  }
  function submit() {
    setTried(true);
    if (!config) return;
    save.mutate({ config, accept: probe.data?.fingerprint ?? null });
  }
  function browse() {
    open({ directory: true, title: "Choose the folder the server runs in" }).then((picked) => {
      if (typeof picked === "string") set({ cwd: picked });
    }, notifyError);
  }

  const stdio = draft.transport === "stdio";
  const tools = probe.data ? parseTools(probe.data.tools) : [];
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{entry ? `Edit ${entry.name}` : "Add MCP server"}</DialogTitle>
          <DialogDescription>
            A server gives dots tools named <code>mcp__name__tool</code>. Dots starts it for each
            run that uses it.
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-3">
          <Field label="Name" htmlFor="server-name" error={shown.name}>
            <Input
              id="server-name"
              value={draft.name}
              disabled={entry !== null}
              onChange={(e) => set({ name: e.target.value })}
            />
          </Field>
          <Field label="Transport" htmlFor="server-transport">
            <NativeSelect
              id="server-transport"
              value={draft.transport}
              onChange={(e) => set({ transport: e.target.value as Draft["transport"] })}
            >
              <option value="stdio">stdio (a program Dots starts)</option>
              <option value="http">http (a server Dots connects to)</option>
            </NativeSelect>
          </Field>
          <p className="text-xs text-muted-foreground">
            Write <code>{"${VAR}"}</code> where a value should come from a variable: {stdio
              ? "Dots reads the variable from Personal (the Env tab) when the server starts, for the dots granted it."
              : "Dots reads the variable from Personal (the Env tab) and sends its value to the server, for the dots granted it. It asks first when the host is new."}
          </p>
          {stdio ? (
            <>
              <Field label="Command" htmlFor="server-command" error={shown.command}>
                <Input
                  id="server-command"
                  className="font-mono"
                  value={draft.command}
                  onChange={(e) => set({ command: e.target.value })}
                />
              </Field>
              <Field
                label="Arguments"
                htmlFor="server-args"
                error={shown.args}
                hint="One argument per line."
              >
                <Textarea
                  id="server-args"
                  rows={3}
                  className="font-mono"
                  value={draft.args}
                  onChange={(e) => set({ args: e.target.value })}
                />
              </Field>
              <Field
                label="Working folder"
                htmlFor="server-cwd"
                error={shown.cwd}
                hint="Empty: the folder the run works in."
              >
                <div className="flex gap-2">
                  <Input
                    id="server-cwd"
                    className="font-mono"
                    value={draft.cwd}
                    onChange={(e) => set({ cwd: e.target.value })}
                  />
                  <Button type="button" variant="outline" onClick={browse}>
                    Browse
                  </Button>
                </div>
              </Field>
              <Field
                label="Environment"
                htmlFor="server-env"
                error={shown.env}
                hint="One KEY=VALUE per line."
              >
                <Textarea
                  id="server-env"
                  rows={3}
                  className="font-mono"
                  value={draft.env}
                  onChange={(e) => set({ env: e.target.value })}
                />
              </Field>
            </>
          ) : (
            <>
              <Field label="URL" htmlFor="server-url" error={shown.url}>
                <Input
                  id="server-url"
                  className="font-mono"
                  placeholder="https://example.com/mcp"
                  value={draft.url}
                  onChange={(e) => set({ url: e.target.value })}
                />
              </Field>
              <Field
                label="Headers"
                htmlFor="server-headers"
                error={shown.headers}
                hint="One Name: value per line."
              >
                <Textarea
                  id="server-headers"
                  rows={3}
                  className="font-mono"
                  value={draft.headers}
                  onChange={(e) => set({ headers: e.target.value })}
                />
              </Field>
            </>
          )}
          {notSet.length > 0 && (
            <p className="text-xs text-amber-300">
              Not in Personal: {notSet.join(", ")}. Add it in the Env tab.
            </p>
          )}
          {secretKeys(draft).map((key) => (
            <p key={key} className="text-xs text-amber-300">
              {key} looks like a secret. When you save, Dots moves its value to Personal as a
              secret and the server reads it from there.
            </p>
          ))}

          <div className="flex flex-col gap-2 rounded-md border p-3">
            <div className="flex items-center gap-3">
              <p className="flex-1 text-xs text-muted-foreground">
                {entry
                  ? "After a change, use Check now to list the tools again."
                  : "Test lists the tools the server offers. Saving then accepts that list."}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={probe.isPending}
                onClick={test}
              >
                {probe.isPending ? "Testing…" : "Test connection"}
              </Button>
            </div>
            {probe.isSuccess && (
              <>
                <p role="status" className="text-xs">
                  {tools.length === 1 ? "1 tool" : `${tools.length} tools`}
                  {entry ? "." : ". Saving accepts this list."}
                </p>
                <ToolList tools={tools} />
              </>
            )}
            {probe.isError &&
              (isDeclined(probe.error) ? (
                <p role="status" className="text-xs text-muted-foreground">
                  You declined. Nothing was tested.
                </p>
              ) : (
                <>
                  <p role="alert" className="text-xs text-destructive">
                    {errorMessage(probe.error)}
                  </p>
                  {!entry && (
                    <p className="text-xs text-muted-foreground">
                      You can still save: the server is saved without its tools, and its tools ask
                      until a check succeeds and you accept the list.
                    </p>
                  )}
                </>
              ))}
          </div>

          {save.isError &&
            (isDeclined(save.error) ? (
              <p role="status" className="text-xs text-muted-foreground">
                You declined. Nothing was saved.
              </p>
            ) : (
              <p role="alert" className="text-xs text-destructive">
                {errorMessage(save.error)}
              </p>
            ))}
        </div>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button type="button" disabled={save.isPending} onClick={submit}>
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
