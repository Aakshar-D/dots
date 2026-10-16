import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import { Plus } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
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
import { errorMessage, notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { isDeclined, ItemRow, plural, RemoveButton, UsedBy, type PluginEntry } from "./parts";

const folderName = (path: string) =>
  path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "";

/// A plugin and its parts, with the switch for its hooks. The core asks the person in a system
/// dialog before it lets the hooks run, so the switch asks nothing itself.
function PluginRow({ entry }: { entry: PluginEntry }) {
  const qc = useQueryClient();
  const hooks = useMutation({
    mutationFn: (run_hooks: boolean) =>
      commands.libraryUpdate(entry.id, {
        kind: "plugin",
        config: { ...entry.config, run_hooks },
      }),
    onSuccess: () => invalidate(qc, keys.library),
    onError: (e) => {
      if (!isDeclined(e)) notifyError(e);
    },
  });
  const detail = entry.plugin;
  const parts = detail
    ? [
        detail.skills.length > 0 && plural(detail.skills.length, "skill"),
        detail.commands.length > 0 && plural(detail.commands.length, "command"),
        detail.agents.length > 0 && plural(detail.agents.length, "agent"),
        detail.servers.length > 0 && plural(detail.servers.length, "MCP server"),
        detail.has_hooks && "hooks",
      ].filter((p): p is string => p !== false)
    : [];
  const switchId = `hooks-${entry.id}`;
  return (
    <ItemRow label={entry.name} actions={<RemoveButton entry={entry} />}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{entry.name}</span>
        {detail?.version && <span className="text-xs text-muted-foreground">{detail.version}</span>}
        {parts.map((part) => (
          <Badge key={part} variant="secondary">
            {part}
          </Badge>
        ))}
      </div>
      {detail?.description && <span className="text-xs">{detail.description}</span>}
      <span className="truncate font-mono text-xs text-muted-foreground">{entry.config.path}</span>
      {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
      {detail?.warnings.map((w) => (
        <span key={w} className="text-xs text-amber-300">
          {w}
        </span>
      ))}
      {(detail?.has_hooks || entry.config.run_hooks) && (
        <div className="flex items-center gap-2 pt-1">
          <Switch
            id={switchId}
            checked={entry.config.run_hooks}
            disabled={hooks.isPending}
            onCheckedChange={(on) => hooks.mutate(on)}
          />
          <Label htmlFor={switchId}>Run this plugin's hooks</Label>
        </div>
      )}
      {hooks.isError && isDeclined(hooks.error) && (
        <span role="status" className="text-xs text-muted-foreground">
          You declined: the hooks stay off.
        </span>
      )}
      <span className="text-xs text-muted-foreground">
        Commands, agents and hooks apply to Claude-engine dots only.
      </span>
      <UsedBy dots={entry.used_by} />
    </ItemRow>
  );
}

function AddPluginDialog(props: { existing: PluginEntry[]; onClose: () => void }) {
  const qc = useQueryClient();
  const installed = useQuery({
    queryKey: keys.installedPlugins,
    queryFn: commands.installedClaudePlugins,
  });
  const [folder, setFolder] = useState<{ path: string; name: string } | null>(null);
  const add = useMutation({
    mutationFn: (v: { name: string; path: string; installed_id: string | null }) =>
      commands.libraryAdd(
        v.name,
        { kind: "plugin", config: { path: v.path, installed_id: v.installed_id, run_hooks: false } },
        null,
      ),
    onSuccess: () => {
      invalidate(qc, keys.library);
      props.onClose();
    },
  });
  function choose() {
    open({ directory: true, title: "Choose a plugin folder" }).then((path) => {
      if (typeof path === "string") setFolder({ path, name: folderName(path) });
    }, notifyError);
  }
  const have = new Set(props.existing.flatMap((p) => [p.name, p.config.installed_id ?? ""]));
  return (
    <Dialog open onOpenChange={(next) => !next && props.onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add plugin</DialogTitle>
          <DialogDescription>
            A plugin brings skills and MCP servers to every dot that uses it. Its commands, agents
            and hooks work in Claude-engine dots only.
          </DialogDescription>
        </DialogHeader>
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold uppercase text-muted-foreground">
            Installed in Claude Code
          </h3>
          {installed.isError && (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(installed.error)}
            </p>
          )}
          {installed.data?.length === 0 && (
            <p className="text-xs text-muted-foreground">No installed plugins found.</p>
          )}
          <ul className="flex flex-col gap-1.5">
            {installed.data?.map((p) => (
              <li
                key={p.id}
                aria-label={p.id}
                className="flex items-center gap-3 rounded-md border p-2"
              >
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="font-medium">
                    {p.name}
                    {p.version && (
                      <span className="ml-2 text-xs text-muted-foreground">{p.version}</span>
                    )}
                  </span>
                  <span className="truncate font-mono text-xs text-muted-foreground">
                    {p.install_path}
                  </span>
                  {!p.exists && <span className="text-xs text-destructive">folder not found</span>}
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!p.exists || add.isPending || have.has(p.id) || have.has(p.name)}
                  onClick={() => add.mutate({ name: p.name, path: p.install_path, installed_id: p.id })}
                >
                  {have.has(p.id) || have.has(p.name) ? "Added" : "Add"}
                </Button>
              </li>
            ))}
          </ul>
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="text-xs font-semibold uppercase text-muted-foreground">From a folder</h3>
          {folder ? (
            <>
              <span className="truncate font-mono text-xs">{folder.path}</span>
              <Field
                label="Name"
                htmlFor="plugin-name"
                hint="The Library name. Its MCP servers are named plugin_<name>_<server>."
              >
                <Input
                  id="plugin-name"
                  value={folder.name}
                  onChange={(e) => setFolder({ ...folder, name: e.target.value })}
                />
              </Field>
              <div>
                <Button
                  size="sm"
                  disabled={add.isPending || folder.name.trim() === ""}
                  onClick={() =>
                    add.mutate({ name: folder.name.trim(), path: folder.path, installed_id: null })
                  }
                >
                  Add from folder
                </Button>
              </div>
            </>
          ) : null}
          <div>
            <Button size="sm" variant="outline" onClick={choose}>
              {folder ? "Choose another folder" : "Choose a folder"}
            </Button>
          </div>
        </section>
        {add.isError &&
          (isDeclined(add.error) ? (
            <p role="status" className="text-xs text-muted-foreground">
              You declined: the plugin was not added.
            </p>
          ) : (
            <p role="alert" className="text-xs text-destructive">
              {errorMessage(add.error)}
            </p>
          ))}
        <DialogFooter>
          <Button type="button" variant="outline" onClick={props.onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/// The plugins tab.
export function PluginsTab({ entries }: { entries: PluginEntry[] }) {
  const [adding, setAdding] = useState(false);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Plugin folders: Claude Code plugins you installed, or any folder with the same layout.
        </p>
        <Button size="sm" onClick={() => setAdding(true)}>
          <Plus aria-hidden /> Add plugin
        </Button>
      </div>
      {entries.length === 0 ? (
        <p className="text-muted-foreground">No plugins yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((entry) => (
            <PluginRow key={entry.id} entry={entry} />
          ))}
        </ul>
      )}
      {adding && <AddPluginDialog existing={entries} onClose={() => setAdding(false)} />}
    </div>
  );
}
