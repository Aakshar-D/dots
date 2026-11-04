import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { AgentImport, ScannedAgent } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { errorMessage } from "@/lib/notify";
import { invalidateDots, keys } from "@/lib/query";
import { PERSONAL, useCurrentScope } from "@/lib/scope";

/// Where an agent lives, as its group's heading says it.
function groupOf(agent: ScannedAgent): string {
  if (agent.source === "project") return `Project ${agent.project ?? ""}`;
  if (agent.source === "plugin") return `Plugin ${agent.plugin ?? ""}`;
  return "Your agents";
}

/// The agents in groups: the user's first, then each project and each plugin as the scan found
/// them.
function grouped(agents: ScannedAgent[]): [string, ScannedAgent[]][] {
  const groups = new Map<string, ScannedAgent[]>();
  for (const agent of agents) {
    const name = groupOf(agent);
    groups.set(name, [...(groups.get(name) ?? []), agent]);
  }
  return [...groups];
}

/// What the agent's dot would run with: its model and its tools.
function summary(agent: ScannedAgent): string {
  const model = agent.model && agent.model !== "inherit" ? agent.model : "default";
  const tools = !agent.tools
    ? "All tools"
    : agent.tools.length === 0
      ? "No tools listed"
      : `Tools ${agent.tools.join(", ")}`;
  return `Model ${model} · ${tools}`;
}

/// Lists the Claude Code agents on this PC (import agents spec section 6) and makes dots of the
/// ones the person ticks, in the account and workspace in view.
export function ImportAgentsDialog({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const scope = useCurrentScope();
  const scan = useQuery({ queryKey: keys.claudeAgents, queryFn: commands.scanClaudeAgents });
  // Untouched agents follow the default: ticked unless they are a dot already.
  const [picked, setPicked] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<AgentImport>();

  const agents = scan.data ?? [];
  const ticked = (agent: ScannedAgent) =>
    agent.error === null && (picked[agent.path] ?? !agent.already_imported);
  const chosen = agents.filter(ticked).map((agent) => agent.path);

  async function importChosen() {
    setBusy(true);
    setError(undefined);
    try {
      const orgId = scope && scope.account !== PERSONAL ? scope.account : null;
      const done = await commands.importClaudeAgents(chosen, orgId, scope?.workspace ?? null);
      setResult(done);
      // What was sent stays unticked: the scan still shows it as not imported until it is
      // read again, and importing it twice would make a second dot.
      setPicked((now) => ({ ...now, ...Object.fromEntries(chosen.map((path) => [path, false])) }));
      const made = done.created.length;
      if (made > 0) toast.success(made === 1 ? "1 dot made" : `${made} dots made`);
      invalidateDots(qc);
      await qc.invalidateQueries({ queryKey: keys.claudeAgents });
    } catch (e) {
      setError(errorMessage(e));
    }
    setBusy(false);
  }

  return (
    <Dialog open onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Import from Claude Code</DialogTitle>
          <DialogDescription>
            Each agent you pick becomes a dot with its prompt, model and tools. Risky tools ask
            before each use.
          </DialogDescription>
        </DialogHeader>
        <div className="flex max-h-[60vh] flex-col gap-4 overflow-auto">
          {scan.isPending && <p className="text-sm text-muted-foreground">Looking for agents…</p>}
          {scan.isError && <ErrorNote error={scan.error} />}
          {scan.isSuccess && agents.length === 0 && (
            <p className="text-sm text-muted-foreground">
              No Claude Code agents were found in your agents folder, your dots' projects or
              your installed plugins.
            </p>
          )}
          {grouped(agents).map(([name, list]) => (
            <fieldset key={name} aria-label={name} className="flex flex-col gap-2">
              <legend className="mb-1 text-xs font-medium text-muted-foreground">{name}</legend>
              {list.map((agent) => (
                <div key={agent.path} className="flex items-start gap-2 rounded-md border p-2">
                  {agent.error === null ? (
                    <input
                      type="checkbox"
                      className="mt-1"
                      aria-label={agent.name}
                      checked={ticked(agent)}
                      onChange={(e) => setPicked({ ...picked, [agent.path]: e.target.checked })}
                    />
                  ) : (
                    <span className="mt-1 w-[13px]" aria-hidden />
                  )}
                  <div className="flex min-w-0 flex-col gap-0.5 text-sm">
                    <span className="flex items-center gap-2">
                      <span className="font-medium">{agent.name}</span>
                      {agent.already_imported && (
                        <span className="text-xs text-muted-foreground">Already a dot</span>
                      )}
                    </span>
                    {agent.error !== null ? (
                      <span className="text-xs text-destructive">{agent.error}</span>
                    ) : (
                      <>
                        {agent.description && (
                          <span className="text-muted-foreground">{agent.description}</span>
                        )}
                        <span className="text-xs text-muted-foreground">{summary(agent)}</span>
                        {agent.warnings.map((warning) => (
                          <span key={warning} className="text-xs text-muted-foreground">
                            {warning}
                          </span>
                        ))}
                      </>
                    )}
                    <span className="truncate text-xs text-muted-foreground" title={agent.path}>
                      {agent.path}
                    </span>
                  </div>
                </div>
              ))}
            </fieldset>
          ))}
          {result && (
            <ul aria-label="Import result" className="flex flex-col gap-1 text-sm">
              {result.created.map((dot) => (
                <li key={dot.id}>Made {dot.name}</li>
              ))}
              {result.failed.map((failure) => (
                <li key={failure.path} className="text-destructive">
                  {failure.path}: {failure.reason}
                </li>
              ))}
            </ul>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
          <Button disabled={busy || chosen.length === 0} onClick={() => void importChosen()}>
            {chosen.length === 1 ? "Import 1 agent" : `Import ${chosen.length} agents`}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
