import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Server } from "lucide-react";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { CatalogStatus } from "@/api/types";
import { EmptyState } from "@/components/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

import { ItemRow, RemoveButton, UsedBy, type PluginEntry, type ServerEntry } from "./parts";
import { ServerForm } from "./ServerForm";

const STATUS: Record<CatalogStatus, { label: string; className: string }> = {
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

function ServerRow(props: {
  entry: ServerEntry;
  owner: string | undefined;
  onEdit: () => void;
  onReview: () => void;
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
            <Button size="sm" onClick={props.onReview}>
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
          {!props.owner && (
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
      <UsedBy dots={entry.used_by} />
    </ItemRow>
  );
}

/// The MCP servers tab. `plugins` names the plugin a server comes with.
export function McpServersTab(props: {
  entries: ServerEntry[];
  plugins: PluginEntry[];
  onReview: (id: string) => void;
}) {
  const [form, setForm] = useState<"new" | ServerEntry | null>(null);
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
              onEdit={() => setForm(entry)}
              onReview={() => props.onReview(entry.id)}
            />
          ))}
        </ul>
      )}
      {form && (
        <ServerForm
          key={form === "new" ? "new" : form.id}
          entry={form === "new" ? null : form}
          onClose={() => setForm(null)}
        />
      )}
    </div>
  );
}
