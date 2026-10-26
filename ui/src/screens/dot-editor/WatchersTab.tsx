import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Eye, Folder, GitPullRequest, Mail } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { useState } from "react";

import { commands } from "@/api/commands";
import type {
  FolderEvent,
  GithubEvent,
  Watcher,
  WatcherConfig,
  WatcherKind,
} from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
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
import { Label } from "@/components/ui/label";
import { NativeSelect } from "@/components/ui/native-select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { localDateTime } from "@/lib/format";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
import { defaultWatcherName, folderInside, watcherState, watcherSummary } from "@/lib/watchers";
import { VariableDialog } from "@/screens/library/EnvTab";

const ICONS = { folder: Folder, email: Mail, github: GitPullRequest } as const;

const ITEM =
  "flex cursor-default items-center gap-2 rounded-sm px-2 py-1.5 outline-none select-none data-[highlighted]:bg-accent";

const TITLES: Record<WatcherKind, string> = {
  folder: "New folder watcher",
  email: "New email watcher",
  github: "New GitHub watcher",
};

const FOLDER_EVENTS: [FolderEvent, string][] = [
  ["created", "Created"],
  ["changed", "Changed"],
  ["deleted", "Deleted"],
];

const GITHUB_EVENTS: [GithubEvent, string][] = [
  ["issue", "Issues"],
  ["pull_request", "Pull requests"],
  ["comment", "Comments"],
  ["push", "Pushes"],
  ["ci", "CI runs"],
];

/// What a new watcher of `kind` starts with.
function blank(kind: WatcherKind): WatcherConfig {
  switch (kind) {
    case "folder":
      return {
        kind,
        path: "",
        pattern: "",
        events: ["created", "changed", "deleted"],
        recursive: true,
      };
    case "email":
      return {
        kind,
        address: "",
        secret: "",
        label: "INBOX",
        from: null,
        subject: null,
        has_attachment: null,
      };
    case "github":
      return {
        kind,
        repos: [],
        events: ["issue", "pull_request"],
        secret: "",
        label: null,
        authors: [],
        skip_bots: true,
        branches: [],
      };
  }
}

/// `text` split by `by`, trimmed, without empty entries.
function list(text: string, by: RegExp): string[] {
  return text
    .split(by)
    .map((s) => s.trim())
    .filter(Boolean);
}

/// `null` for a blank text, else the text.
const orNull = (text: string) => (text.trim() === "" ? null : text);

/// The option of the vault chooser that opens the vault's form.
const ADD = "\u0000add";

/// The secret entries of the vault, "Add to vault…" opening the vault's form (watchers spec
/// section 8).
function VaultChooser(props: {
  id: string;
  value: string;
  onChange: (name: string) => void;
  initialName: string;
}) {
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const [adding, setAdding] = useState(false);
  const secrets = (env.data ?? []).filter((e) => e.kind === "secret").map((e) => e.name);
  // What was just saved may not be in the list yet: it is an option all the same.
  if (props.value && !secrets.includes(props.value)) secrets.push(props.value);
  return (
    <>
      <NativeSelect
        id={props.id}
        value={props.value}
        onChange={(e) => {
          if (e.target.value === ADD) setAdding(true);
          else props.onChange(e.target.value);
        }}
      >
        <option value="">Choose a vault entry</option>
        {secrets.map((name) => (
          <option key={name} value={name}>
            {name}
          </option>
        ))}
        <option value={ADD}>Add to vault…</option>
      </NativeSelect>
      {adding && (
        <VariableDialog
          entry={null}
          initialName={props.initialName}
          onSaved={(saved) => props.onChange(saved.name)}
          onClose={() => setAdding(false)}
        />
      )}
    </>
  );
}

/// The form of a new watcher of `kind`, or of `watcher` (spec section 8). The core checks it;
/// its refusal shows here and keeps the form open.
function WatcherForm(props: {
  dotId: string;
  workdir: string | null;
  watcher: Watcher | null;
  kind: WatcherKind;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [config, setConfig] = useState<WatcherConfig>(
    props.watcher?.config ?? blank(props.kind),
  );
  const [name, setName] = useState(props.watcher?.name ?? "");
  const [named, setNamed] = useState(props.watcher !== null);
  // The list fields as typed, so a trailing comma or line stays while the person writes.
  const github = config.kind === "github" ? config : null;
  const [repos, setRepos] = useState(github?.repos.join("\n") ?? "");
  const [authors, setAuthors] = useState(github?.authors.join(", ") ?? "");
  const [branches, setBranches] = useState(github?.branches.join(", ") ?? "");
  const shownName = named ? name : defaultWatcherName(config);
  const save = useMutation({
    mutationFn: (c: WatcherConfig) =>
      props.watcher
        ? commands.watcherUpdate(props.watcher.id, shownName, c)
        : commands.watcherAdd(props.dotId, shownName, c),
    onSuccess: () => props.onClose(),
    onSettled: () => {
      invalidate(qc, keys.watchers(props.dotId));
      invalidate(qc, keys.inbox);
      invalidate(qc, keys.overview);
    },
  });

  function submit() {
    if (config.kind !== "github") return save.mutate(config);
    save.mutate({
      ...config,
      repos: list(repos, /\r?\n/),
      authors: list(authors, /,/),
      branches: list(branches, /,/),
    });
  }

  function browse() {
    open({ directory: true, title: "Choose the folder to watch" }).then((picked) => {
      if (typeof picked === "string") {
        setConfig((c) => (c.kind === "folder" ? { ...c, path: picked } : c));
      }
    }, notifyError);
  }

  const setSecret = (secret: string) =>
    setConfig((c) => (c.kind === "folder" ? c : { ...c, secret }));
  const title = props.watcher ? `Edit ${props.watcher.name}` : TITLES[props.kind];
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            A watcher starts this dot when what it watches changes; each event is one run.
          </DialogDescription>
        </DialogHeader>
        <div className="flex max-h-[60vh] flex-col gap-3 overflow-y-auto pr-1">
          <Field label="Name" htmlFor="watcher-name">
            <Input
              id="watcher-name"
              value={shownName}
              onChange={(e) => {
                setNamed(true);
                setName(e.target.value);
              }}
            />
          </Field>
          {config.kind === "folder" && (
            <>
              <Field
                label="Folder"
                htmlFor="watcher-folder"
                hint={
                  props.workdir &&
                  config.path.trim() !== "" &&
                  !folderInside(config.path, props.workdir)
                    ? `The dot can only open files inside its folder (${props.workdir}); this folder is outside it.`
                    : undefined
                }
              >
                <div className="flex gap-2">
                  <Input
                    id="watcher-folder"
                    value={config.path}
                    onChange={(e) => setConfig({ ...config, path: e.target.value })}
                  />
                  <Button type="button" variant="outline" onClick={browse}>
                    Browse…
                  </Button>
                </div>
              </Field>
              <Field
                label="Pattern"
                htmlFor="watcher-pattern"
                hint="A file name pattern, e.g. *.pdf; empty for every file."
              >
                <Input
                  id="watcher-pattern"
                  value={config.pattern}
                  onChange={(e) => setConfig({ ...config, pattern: e.target.value })}
                />
              </Field>
              <div className="flex flex-wrap gap-4">
                {FOLDER_EVENTS.map(([event, label]) => (
                  <label key={event} className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={config.events.includes(event)}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          events: FOLDER_EVENTS.map(([x]) => x).filter((x) =>
                            x === event ? e.target.checked : config.events.includes(x),
                          ),
                        })
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
              <div className="flex items-center gap-2">
                <Switch
                  id="watcher-recursive"
                  checked={config.recursive}
                  onCheckedChange={(v) => setConfig({ ...config, recursive: v })}
                />
                <Label htmlFor="watcher-recursive">Subfolders</Label>
              </div>
            </>
          )}
          {config.kind === "email" && (
            <>
              <p className="text-sm text-muted-foreground">
                Gmail needs an app password, which your Google account offers once 2-step
                verification is on.{" "}
                <a
                  href="https://myaccount.google.com/apppasswords"
                  target="_blank"
                  rel="noreferrer"
                  className="text-foreground underline"
                >
                  Make an app password
                </a>
              </p>
              <Field label="Gmail address" htmlFor="watcher-address">
                <Input
                  id="watcher-address"
                  value={config.address}
                  onChange={(e) => setConfig({ ...config, address: e.target.value })}
                />
              </Field>
              <Field label="App password" htmlFor="watcher-secret">
                <VaultChooser
                  id="watcher-secret"
                  value={config.secret}
                  initialName="GMAIL_APP_PASSWORD"
                  onChange={setSecret}
                />
              </Field>
              <Field label="Gmail label" htmlFor="watcher-label">
                <Input
                  id="watcher-label"
                  value={config.label}
                  onChange={(e) => setConfig({ ...config, label: e.target.value })}
                />
              </Field>
              <Field label="From" htmlFor="watcher-from">
                <Input
                  id="watcher-from"
                  value={config.from ?? ""}
                  onChange={(e) => setConfig({ ...config, from: orNull(e.target.value) })}
                />
              </Field>
              <Field label="Subject contains" htmlFor="watcher-subject">
                <Input
                  id="watcher-subject"
                  value={config.subject ?? ""}
                  onChange={(e) => setConfig({ ...config, subject: orNull(e.target.value) })}
                />
              </Field>
              <Field label="Attachments" htmlFor="watcher-attachments">
                <NativeSelect
                  id="watcher-attachments"
                  value={String(config.has_attachment)}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      has_attachment:
                        e.target.value === "null" ? null : e.target.value === "true",
                    })
                  }
                >
                  <option value="null">Any</option>
                  <option value="true">With attachments</option>
                  <option value="false">Without attachments</option>
                </NativeSelect>
              </Field>
            </>
          )}
          {config.kind === "github" && (
            <>
              <Field label="Repositories" htmlFor="watcher-repos" hint="One owner/repo per line.">
                <Textarea
                  id="watcher-repos"
                  rows={3}
                  value={repos}
                  onChange={(e) => {
                    setRepos(e.target.value);
                    setConfig({ ...config, repos: list(e.target.value, /\r?\n/) });
                  }}
                />
              </Field>
              <div className="flex flex-wrap gap-4">
                {GITHUB_EVENTS.map(([event, label]) => (
                  <label key={event} className="flex items-center gap-2 text-sm">
                    <input
                      type="checkbox"
                      checked={config.events.includes(event)}
                      onChange={(e) =>
                        setConfig({
                          ...config,
                          events: GITHUB_EVENTS.map(([x]) => x).filter((x) =>
                            x === event ? e.target.checked : config.events.includes(x),
                          ),
                        })
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
              <Field label="Token" htmlFor="watcher-token">
                <VaultChooser
                  id="watcher-token"
                  value={config.secret}
                  initialName="GITHUB_TOKEN"
                  onChange={setSecret}
                />
              </Field>
              <Field label="Label" htmlFor="watcher-gh-label">
                <Input
                  id="watcher-gh-label"
                  value={config.label ?? ""}
                  onChange={(e) => setConfig({ ...config, label: orNull(e.target.value) })}
                />
              </Field>
              <Field label="Authors" htmlFor="watcher-authors" hint="Comma-separated logins.">
                <Input
                  id="watcher-authors"
                  value={authors}
                  onChange={(e) => setAuthors(e.target.value)}
                />
              </Field>
              <div className="flex items-center gap-2">
                <Switch
                  id="watcher-bots"
                  checked={config.skip_bots}
                  onCheckedChange={(v) => setConfig({ ...config, skip_bots: v })}
                />
                <Label htmlFor="watcher-bots">Skip bots</Label>
              </div>
              <Field
                label="Branches"
                htmlFor="watcher-branches"
                hint="Comma-separated; for pushes and CI runs."
              >
                <Input
                  id="watcher-branches"
                  value={branches}
                  onChange={(e) => setBranches(e.target.value)}
                />
              </Field>
            </>
          )}
        </div>
        {save.error && (
          <p role="alert" className="text-sm text-destructive">
            {errorMessage(save.error)}
          </p>
        )}
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

/// The dot's Watchers tab (watchers spec section 8): its watchers, each with what it watches,
/// its last event and its state, a switch, Edit and Delete; Add watcher by kind.
export function WatchersTab({ dotId }: { dotId: string }) {
  const qc = useQueryClient();
  const list = useQuery({
    queryKey: keys.watchers(dotId),
    queryFn: () => commands.watchersList(dotId),
  });
  const dot = useQuery({ queryKey: keys.dot(dotId), queryFn: () => commands.getDot(dotId) });
  const [form, setForm] = useState<{ kind: WatcherKind; watcher: Watcher | null } | null>(null);
  const refresh = () => {
    invalidate(qc, keys.watchers(dotId));
    invalidate(qc, keys.inbox);
    invalidate(qc, keys.overview);
  };
  const toggle = useMutation({
    mutationFn: async (v: { watcher: Watcher; on: boolean }) => {
      const { watcher, on } = v;
      if (!on) return commands.watcherSetEnabled(watcher.id, false);
      if (watcher.paused_reason !== null) {
        const resumed = await commands.watcherResume(watcher.id);
        return watcher.enabled ? resumed : commands.watcherSetEnabled(watcher.id, true);
      }
      return commands.watcherSetEnabled(watcher.id, true);
    },
    onError: notifyError,
    onSettled: refresh,
  });
  const remove = useMutation({
    mutationFn: (id: string) => commands.watcherDelete(id),
    onError: notifyError,
    onSettled: refresh,
  });

  if (list.isPending) return null;
  if (list.isError) return <ErrorNote error={list.error} />;
  const watchers = list.data;
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-end">
        <DropdownMenu.Root modal={false}>
          <DropdownMenu.Trigger asChild>
            <Button size="sm">Add watcher</Button>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content
              align="end"
              className="z-50 w-40 rounded-md border bg-popover p-1 text-sm text-popover-foreground shadow-md"
            >
              {(["folder", "email", "github"] as const).map((kind) => {
                const Icon = ICONS[kind];
                return (
                  <DropdownMenu.Item
                    key={kind}
                    className={ITEM}
                    onSelect={() => setForm({ kind, watcher: null })}
                  >
                    <Icon aria-hidden className="size-4" />
                    {{ folder: "Folder", email: "Email", github: "GitHub" }[kind]}
                  </DropdownMenu.Item>
                );
              })}
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
      {watchers.length === 0 ? (
        <EmptyState icon={Eye}>
          No watchers. A watcher starts this dot when a file changes, an email arrives or
          something happens on GitHub.
        </EmptyState>
      ) : (
        <ul aria-label="Watchers" className="flex flex-col gap-2">
          {watchers.map((w) => {
            const Icon = ICONS[w.config.kind];
            const paused = w.paused_reason !== null;
            return (
              <li
                key={w.id}
                aria-label={w.name}
                className="flex items-start gap-3 rounded-lg border p-3"
              >
                <Icon aria-hidden className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                  <p className="font-medium">{w.name}</p>
                  <p className="truncate text-sm text-muted-foreground">{watcherSummary(w)}</p>
                  <p className="text-xs text-muted-foreground">
                    {w.last_event_at
                      ? `Last event ${localDateTime(w.last_event_at)}`
                      : "No event yet"}
                  </p>
                  <p
                    className={paused ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
                  >
                    {watcherState(w)}
                  </p>
                </div>
                <div className="flex h-8 items-center gap-2">
                  <Switch
                    id={`watcher-on-${w.id}`}
                    aria-label={`${w.name} is on`}
                    checked={w.enabled && !paused}
                    disabled={toggle.isPending}
                    onCheckedChange={(on) => toggle.mutate({ watcher: w, on })}
                  />
                  <Label htmlFor={`watcher-on-${w.id}`}>On</Label>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setForm({ kind: w.config.kind, watcher: w })}
                >
                  Edit
                </Button>
                <ConfirmButton
                  title={`Delete the watcher ${w.name}?`}
                  description="It stops starting this dot. Runs it started stay."
                  confirmLabel="Delete"
                  disabled={remove.isPending}
                  onConfirm={() => remove.mutate(w.id)}
                >
                  Delete
                </ConfirmButton>
              </li>
            );
          })}
        </ul>
      )}
      {form && (
        <WatcherForm
          dotId={dotId}
          workdir={dot.data?.workdir ?? null}
          watcher={form.watcher}
          kind={form.kind}
          onClose={() => setForm(null)}
        />
      )}
    </div>
  );
}
