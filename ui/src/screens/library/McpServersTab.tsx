import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Server } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { CatalogStatus, ScopedCatalog } from "@/api/types";
import { EmptyState } from "@/components/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import {
  ItemRow,
  PERSONAL,
  RemoveButton,
  UsedBy,
  useScopes,
  type PluginEntry,
  type ServerEntry,
} from "./parts";
import { ServerForm } from "./ServerForm";

export const STATUS: Record<CatalogStatus, { label: string; className: string }> = {
  accepted: { label: "Accepted", className: "border-success/40 bg-transparent text-success" },
  changed: { label: "Tools changed", className: "border-needs-border bg-transparent text-needs" },
  unreachable: {
    label: "Unreachable",
    className: "border-destructive/40 bg-transparent text-destructive",
  },
  unchecked: { label: "Unchecked", className: "bg-muted text-muted-foreground" },
};

/// An MCP server's status: its catalog's, or "Unchecked" when it has none.
export function serverStatus(entry: ServerEntry): CatalogStatus {
  return entry.catalog?.status ?? "unchecked";
}

/// The orgs a server is listed in beside Personal: each one where its catalog is stored (a
/// Check or a run made it, even one reset to unchecked since) or whose dots use the server. An
/// org that has neither has nothing to show.
function orgCatalogs(entry: ServerEntry): ScopedCatalog[] {
  return entry.catalogs.filter((c) => c.scope !== PERSONAL && (c.stored || c.used_by.length > 0));
}

/// A server's catalog in one org: its status, its Review, its error and the org's dots that use
/// it. Keyed by (server, scope).
function ScopeLine(props: { entry: ServerEntry; scoped: ScopedCatalog; onReview: () => void }) {
  const { entry, scoped } = props;
  const { nameOf } = useScopes();
  const name = nameOf(scoped.scope);
  const { label, className } = STATUS[scoped.catalog.status];
  return (
    <li
      aria-label={`${entry.name} in ${name}`}
      className="flex flex-wrap items-center gap-2 rounded-md border px-2 py-1.5"
    >
      <span className="text-xs">{`In ${name}`}</span>
      <Badge variant="secondary" className={className}>
        {label}
      </Badge>
      {scoped.used_by.length > 0 && <UsedBy dots={scoped.used_by} />}
      {scoped.catalog.status === "unreachable" && scoped.catalog.error && (
        <span className="text-xs text-destructive">{scoped.catalog.error}</span>
      )}
      {scoped.catalog.status === "changed" && (
        <Button size="sm" className="ml-auto" onClick={props.onReview}>
          Review
        </Button>
      )}
    </li>
  );
}

function ServerRow(props: {
  entry: ServerEntry;
  owner: string | undefined;
  onEdit: () => void;
  onReview: (scope?: string) => void;
}) {
  const { entry } = props;
  const qc = useQueryClient();
  const check = useMutation({
    mutationFn: () => commands.libraryCheck(entry.id),
    onSuccess: () => invalidate(qc, keys.library),
    onError: notifyError,
  });
  const status = serverStatus(entry);
  const { label, className } = STATUS[status];
  const config = entry.config;
  return (
    <ItemRow
      label={entry.name}
      actions={
        <>
          {status === "changed" && (
            <Button size="sm" onClick={() => props.onReview()}>
              Review
            </Button>
          )}
          <Button
            size="sm"
            variant="outline"
            disabled={check.isPending}
            onClick={() => check.mutate()}
          >
            {check.isPending ? "Checking…" : "Check now"}
          </Button>
          {props.owner ? (
            // Read-only, for Check as and the origins.
            <Button size="sm" variant="outline" onClick={props.onEdit}>
              Details
            </Button>
          ) : (
            <>
              <Button size="sm" variant="outline" onClick={props.onEdit}>
                Edit
              </Button>
              <RemoveButton entry={entry} />
            </>
          )}
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium">{entry.name}</span>
        <Badge variant="outline">{config.transport}</Badge>
        <Badge variant="secondary" className={className}>
          {label}
        </Badge>
        {entry.missing_variables.length > 0 && (
          <Badge variant="secondary" className="border-needs-border bg-transparent text-needs">
            {`Missing variable: ${entry.missing_variables.join(", ")}`}
          </Badge>
        )}
      </div>
      <span className="truncate font-mono text-xs text-muted-foreground">
        {config.transport === "stdio" ? [config.command, ...config.args].join(" ") : config.url}
      </span>
      {status === "unreachable" && entry.catalog?.error && (
        <span className="text-xs text-destructive">{entry.catalog.error}</span>
      )}
      {props.owner ? (
        <span className="text-xs text-muted-foreground">{`Comes with plugin ${props.owner}`}</span>
      ) : null}
      {/* Personal's own dots: each org's are on its line below. */}
      <UsedBy dots={entry.catalogs.find((c) => c.scope === PERSONAL)?.used_by ?? []} />
      {orgCatalogs(entry).length > 0 && (
        <ul className="flex flex-col gap-1.5">
          {orgCatalogs(entry).map((scoped) => (
            <ScopeLine
              key={`${entry.id}/${scoped.scope}`}
              entry={entry}
              scoped={scoped}
              onReview={() => props.onReview(scoped.scope)}
            />
          ))}
        </ul>
      )}
    </ItemRow>
  );
}

/// The MCP servers tab: each server once, with its Personal state, then its state in each org
/// it is checked or used in. `plugins` names the plugin a server comes with. `onReview` opens
/// the review of a changed list in `scope`, an org id; left out, Personal's.
export function McpServersTab(props: {
  entries: ServerEntry[];
  plugins: PluginEntry[];
  onReview: (id: string, scope?: string) => void;
}) {
  // The server being edited is kept by id and looked up in `entries` on each render, so a
  // refetch (a confirmed origin, a check) shows in the open dialog.
  const [form, setForm] = useState<"new" | string | null>(null);
  const editing = form && form !== "new" ? props.entries.find((e) => e.id === form) : undefined;
  const owners = new Map(props.plugins.map((p) => [p.id, p.name]));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <p className="flex-1 text-xs text-muted-foreground">
          Tools that dots can call. Each server's tool list is accepted by you and checked at
          every run.
        </p>
        <Button size="sm" onClick={() => setForm("new")}>
          <Plus aria-hidden /> Add server
        </Button>
      </div>
      {props.entries.length === 0 ? (
        <EmptyState icon={Server}>No MCP servers yet.</EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {props.entries.map((entry) => (
            <ServerRow
              key={entry.id}
              entry={entry}
              owner={entry.owner_id ? (owners.get(entry.owner_id) ?? "a plugin") : undefined}
              onEdit={() => setForm(entry.id)}
              onReview={(scope) => props.onReview(entry.id, scope)}
            />
          ))}
        </ul>
      )}
      {form && (form === "new" || editing) && (
        <ServerForm
          key={form}
          entry={editing ?? null}
          onClose={() => setForm(null)}
        />
      )}
    </div>
  );
}
