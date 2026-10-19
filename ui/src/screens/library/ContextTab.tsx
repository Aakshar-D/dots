import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
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
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import {
  ItemRow,
  plural,
  RemoveButton,
  UsedBy,
  type ContextEntry,
} from "./parts";

/// The longest name and description, as `library::check_name` and `LibraryConfig::validate`
/// take them.
const MAX_NAME = 40;
const MAX_DESCRIPTION = 300;
const NAME_RULE =
  "Use 1 to 40 of a-z, 0-9, _ and -, starting with a letter or digit.";

export const validName = (name: string) =>
  /^[a-z0-9][a-z0-9_-]{0,39}$/.test(name);

/// A name for a collection from its folder: lower case, other characters as `-`, cut at 40, no
/// `-` or `_` at either end.
export function nameFromFolder(path: string): string {
  const folder =
    path
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .pop() ?? "";
  return folder
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^[-_]+/, "")
    .slice(0, MAX_NAME)
    .replace(/[-_]+$/, "");
}

type Form = {
  name: string;
  path: string;
  description: string;
  every_dot: boolean;
};

/// Add a collection (`entry` null) or change one: its folder, description and "Every dot". The
/// name of a stored collection stays.
function CollectionDialog(props: {
  entry: ContextEntry | null;
  path: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { entry } = props;
  const [form, setForm] = useState<Form>({
    name: entry?.name ?? nameFromFolder(props.path),
    path: props.path,
    description: entry?.config.description ?? "",
    every_dot: entry?.config.every_dot ?? false,
  });
  const [tried, setTried] = useState(false);
  const [state, setState] = useState<{ busy: boolean; error?: string }>({
    busy: false,
  });
  const errors = {
    name: validName(form.name) ? undefined : NAME_RULE,
    description:
      [...form.description.trim()].length > MAX_DESCRIPTION
        ? `At most ${MAX_DESCRIPTION} characters.`
        : undefined,
  };
  const set = (change: Partial<Form>) => setForm({ ...form, ...change });

  async function browse() {
    try {
      const picked = await open({
        directory: true,
        title: "Choose the collection's folder",
      });
      if (typeof picked === "string") set({ path: picked });
    } catch (e) {
      notifyError(e);
    }
  }

  async function save() {
    setTried(true);
    if (errors.name || errors.description) return;
    const config = {
      kind: "context" as const,
      config: {
        path: form.path,
        description: form.description.trim(),
        every_dot: form.every_dot,
      },
    };
    setState({ busy: true });
    try {
      if (entry) await commands.libraryUpdate(entry.id, config);
      else await commands.libraryAdd(form.name, config, null);
      invalidate(qc, keys.library);
      props.onClose();
    } catch (e) {
      setState({ busy: false, error: errorMessage(e) });
    }
  }

  const title = entry ? `Edit ${entry.name}` : "Add context collection";
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Dots read these files as guidance. Only add folders whose writers
            you trust.
          </DialogDescription>
        </DialogHeader>
        <Field
          label="Name"
          htmlFor="context-name"
          error={tried ? errors.name : undefined}
        >
          <Input
            id="context-name"
            value={form.name}
            disabled={entry !== null}
            onChange={(e) => set({ name: e.target.value })}
          />
        </Field>
        <Field label="Folder" htmlFor="context-path">
          <div className="flex gap-2">
            <Input
              id="context-path"
              readOnly
              className="font-mono"
              value={form.path}
            />
            <Button
              type="button"
              variant="outline"
              onClick={() => void browse()}
            >
              Choose…
            </Button>
          </div>
        </Field>
        <Field
          label="Description"
          htmlFor="context-description"
          error={tried ? errors.description : undefined}
          hint="Dots see it next to the name, to know when to look here."
        >
          <Textarea
            id="context-description"
            rows={2}
            value={form.description}
            onChange={(e) => set({ description: e.target.value })}
          />
        </Field>
        <div className="flex items-center gap-2">
          <Switch
            id="context-every-dot"
            checked={form.every_dot}
            onCheckedChange={(every_dot) => set({ every_dot })}
          />
          <Label htmlFor="context-every-dot">Every dot reads it</Label>
        </div>
        {state.error && (
          <p role="alert" className="text-xs text-destructive">
            {state.error}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={state.busy}
            onClick={() => void save()}
          >
            Save
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The first documents of a collection, each with its title and summary.
function DocumentsDialog(props: { entry: ContextEntry; onClose: () => void }) {
  const docs = useQuery({
    queryKey: [...keys.library, "documents", props.entry.id],
    queryFn: () => commands.contextDocuments(props.entry.id),
  });
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{`Documents of ${props.entry.name}`}</DialogTitle>
          <DialogDescription className="font-mono">
            {props.entry.config.path}
          </DialogDescription>
        </DialogHeader>
        {docs.isPending && (
          <p className="text-muted-foreground">Reading the folder…</p>
        )}
        {docs.isError && <ErrorNote error={docs.error} />}
        {docs.data && docs.data.documents.length === 0 && (
          <p className="text-muted-foreground">No documents in that folder.</p>
        )}
        {docs.data && docs.data.documents.length > 0 && (
          <ul className="flex flex-col gap-1.5">
            {docs.data.documents.map((doc) => (
              <li
                key={doc.path}
                className="flex flex-col rounded-md border px-2 py-1.5"
              >
                <span className="text-sm font-medium">{doc.title}</span>
                {doc.summary && (
                  <span className="text-xs text-muted-foreground">
                    {doc.summary}
                  </span>
                )}
                <span className="font-mono text-xs text-muted-foreground">
                  {doc.path}
                </span>
              </li>
            ))}
          </ul>
        )}
        {docs.data?.truncated && (
          <p role="status" className="text-xs text-muted-foreground">
            {`The first ${docs.data.documents.length} of ${docs.data.complete ? "" : "at least "}${plural(docs.data.total, "document")}.`}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}

type Open =
  | { what: "add"; path: string }
  | { what: "edit"; entry: ContextEntry }
  | { what: "documents"; entry: ContextEntry };

/// The context tab: folders of documents that dots read with the `context` tool.
export function ContextTab({ entries }: { entries: ContextEntry[] }) {
  const [opened, setOpened] = useState<Open | null>(null);

  async function add() {
    try {
      const folder = await open({
        directory: true,
        title: "Choose the collection's folder",
      });
      if (typeof folder === "string") setOpened({ what: "add", path: folder });
    } catch (e) {
      notifyError(e);
    }
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Folders of text documents (Markdown, text, JSON, YAML, CSV) that dots
          list, search and read. Dots never change them.
        </p>
        <Button size="sm" onClick={() => void add()}>
          <Plus aria-hidden /> Add collection
        </Button>
      </div>
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No context collections yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <ItemRow
              key={entry.id}
              label={entry.name}
              actions={
                <>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setOpened({ what: "documents", entry })}
                  >
                    Documents
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setOpened({ what: "edit", entry })}
                  >
                    Edit
                  </Button>
                  <RemoveButton entry={entry} />
                </>
              }
            >
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium">{entry.name}</span>
                {entry.config.every_dot && (
                  <Badge variant="secondary">every dot</Badge>
                )}
                {entry.context && (
                  <span className="text-xs text-muted-foreground">
                    {entry.context.complete ? "" : "at least "}
                    {plural(entry.context.documents, "document")}
                  </span>
                )}
              </div>
              {entry.config.description && (
                <span className="text-xs">{entry.config.description}</span>
              )}
              <span className="truncate font-mono text-xs text-muted-foreground">
                {entry.config.path}
              </span>
              {entry.problem && (
                <span className="text-xs text-destructive">
                  {entry.problem}
                </span>
              )}
              {entry.config.every_dot ? (
                <span className="text-xs text-muted-foreground">
                  Every dot reads it
                </span>
              ) : (
                <UsedBy dots={entry.used_by} />
              )}
            </ItemRow>
          ))}
        </ul>
      )}
      {opened?.what === "add" && (
        <CollectionDialog
          entry={null}
          path={opened.path}
          onClose={() => setOpened(null)}
        />
      )}
      {opened?.what === "edit" && (
        <CollectionDialog
          entry={opened.entry}
          path={opened.entry.config.path}
          onClose={() => setOpened(null)}
        />
      )}
      {opened?.what === "documents" && (
        <DocumentsDialog entry={opened.entry} onClose={() => setOpened(null)} />
      )}
    </div>
  );
}
