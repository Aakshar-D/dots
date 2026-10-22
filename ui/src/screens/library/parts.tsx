import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { ReactNode } from "react";

import { CommandError, commands } from "@/api/commands";
import type { LibraryEntry, McpTool } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { Badge } from "@/components/ui/badge";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";

import { isReadOnly } from "./tools";

/// Whether an error is the person's own "no" to a native confirmation (a `cancelled` command).
/// The Library screen never asks a second time: the core's dialog is the question.
export function isDeclined(e: unknown): boolean {
  return e instanceof CommandError && e.kind === "cancelled";
}

export type ServerEntry = Extract<LibraryEntry, { kind: "mcp_server" }>;
export type SkillEntry = Extract<LibraryEntry, { kind: "skill" }>;
export type PluginEntry = Extract<LibraryEntry, { kind: "plugin" }>;
export type ContextEntry = Extract<LibraryEntry, { kind: "context" }>;

export const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

/// One item of a list: `children` on the left, the actions on the right.
export function ItemRow(props: { label: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <li aria-label={props.label} className="flex items-start gap-3 rounded-lg border p-3">
      <div className="flex min-w-0 flex-1 flex-col gap-1">{props.children}</div>
      <div className="flex shrink-0 items-center gap-2">{props.actions}</div>
    </li>
  );
}

/// The dots that use an item.
export function UsedBy({ dots }: { dots: LibraryEntry["used_by"] }) {
  if (dots.length === 0) return <span className="text-xs text-muted-foreground">Not used yet</span>;
  return (
    <span className="text-xs text-muted-foreground">
      Used by {dots.map((d) => d.name).join(", ")}
    </span>
  );
}

/// An MCP server's tools, each with its description and a "read-only" badge where the server
/// says so.
export function ToolList({ tools }: { tools: McpTool[] }) {
  if (tools.length === 0) {
    return <p className="text-xs text-muted-foreground">The server lists no tools.</p>;
  }
  return (
    <ul className="flex flex-col gap-1.5">
      {tools.map((tool) => (
        <li key={tool.name} className="flex flex-col gap-0.5 rounded-md border px-2 py-1.5">
          <div className="flex items-center gap-2">
            <span className="font-mono text-xs font-medium">{tool.name}</span>
            {isReadOnly(tool) && <Badge variant="secondary">read-only</Badge>}
          </div>
          {tool.description && <span className="text-xs text-muted-foreground">{tool.description}</span>}
        </li>
      ))}
    </ul>
  );
}

const KIND_WORD = {
  mcp_server: "server",
  skill: "skill",
  plugin: "plugin",
  context: "collection",
} as const;

/// "Remove": asks first and names the dots that use the item, then removes it and takes it out
/// of them (`confirm` is true only when dots use it).
export function RemoveButton({ entry }: { entry: LibraryEntry }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.libraryRemove(entry.id, entry.used_by.length > 0),
    onSuccess: () => {
      invalidate(qc, keys.library);
      invalidateDots(qc);
    },
    onError: notifyError,
  });
  const word = KIND_WORD[entry.kind];
  const everyDot = entry.kind === "context" && entry.config.every_dot;
  const users = entry.used_by.map((d) => d.name).join(", ");
  return (
    <ConfirmButton
      title={`Remove ${entry.name}?`}
      description={
        <>
          {everyDot && "Every dot reads it. "}
          {entry.used_by.length > 0
            ? `Used by ${users}. Removing it takes it out of them.`
            : !everyDot && "No dot uses it."}
          {entry.kind === "plugin" && " Its MCP servers go with it."}
          {entry.dots_folder && " Its folder in Dots' skills folder is deleted too."}
        </>
      }
      confirmLabel={`Remove ${word}`}
      disabled={remove.isPending}
      onConfirm={() => remove.mutate()}
    >
      Remove
    </ConfirmButton>
  );
}
