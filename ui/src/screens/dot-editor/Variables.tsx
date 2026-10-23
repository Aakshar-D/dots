import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { commands } from "@/api/commands";
import type { LibraryEntry, Uses } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { keys } from "@/lib/query";
import { validVariableName } from "@/screens/library/EnvTab";
import { variablesIn } from "@/screens/library/text";

type Grants = Record<string, string[]>;
type Server = Extract<LibraryEntry, { kind: "mcp_server" }>;

const SHELL = "shell";
const has = (names: string[], name: string) =>
  names.some((n) => n.toLowerCase() === name.toLowerCase());

/// The Library servers a dot with `uses` starts: those it ticks, and those of its plugins.
function usedServers(library: LibraryEntry[], uses: Uses | null): Server[] {
  if (!uses) return [];
  const plugins = new Set(
    library.filter((e) => e.kind === "plugin" && uses.plugins.includes(e.name)).map((e) => e.id),
  );
  return library.filter(
    (e): e is Server =>
      e.kind === "mcp_server" &&
      (e.owner_id === null ? uses.mcp_servers.includes(e.name) : plugins.has(e.owner_id)),
  );
}

/// The `${VAR}` names a server's config reads.
function wants(server: Server): string[] {
  const c = server.config;
  return c.transport === "stdio"
    ? variablesIn([...Object.values(c.env), ...c.args, c.cwd ?? ""])
    : variablesIn([c.url, ...Object.values(c.headers)]);
}

/// A dot's grants (env vault spec §7): what each server it uses may read, and what its shell
/// gets. A server the dot did not use before (`before` is the stored dot's `uses`) is granted
/// its variables when the dot is saved; a variable a server asks for later waits for Grant.
export function Variables(props: {
  uses: Uses;
  variables: Grants;
  before: Uses | null;
  onChange: (variables: Grants) => void;
}) {
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const [shellName, setShellName] = useState("");
  const [tried, setTried] = useState(false);
  if (library.isError) return <ErrorNote error={library.error} />;
  const entries = library.data ?? [];
  const personal = (env.data ?? []).map((e) => e.name);
  const had = usedServers(entries, props.before).map((s) => s.name);
  const shell = props.variables[SHELL] ?? [];
  const grant = (key: string, name: string) =>
    props.onChange({ ...props.variables, [key]: [...(props.variables[key] ?? []), name] });
  const missing = (name: string) =>
    env.data && !has(personal, name) ? " · not in Personal" : "";

  function addShell() {
    setTried(true);
    if (!validVariableName(shellName) || has(shell, shellName)) return;
    grant(SHELL, shellName);
    setShellName("");
    setTried(false);
  }

  return (
    <div className="flex flex-col gap-3">
      {usedServers(entries, props.uses).map((server) => {
        const key = `mcp:${server.name}`;
        const granted = props.variables[key] ?? [];
        const fresh = !had.includes(server.name);
        // A name Personal could never hold is not offered.
        const names = wants(server).filter(validVariableName);
        if (names.length === 0 && granted.length === 0) return null;
        return (
          <div key={server.id} role="group" aria-label={server.name} className="flex flex-col gap-1">
            <span className="font-mono text-sm font-medium">{server.name}</span>
            {[...new Set([...granted, ...names])].map((name) =>
              has(granted, name) ? (
                <span key={name} className="font-mono text-xs">
                  {name}
                  <span className="text-muted-foreground">{missing(name)}</span>
                </span>
              ) : fresh ? (
                <span key={name} className="font-mono text-xs text-muted-foreground">
                  {name} · granted when you save
                </span>
              ) : (
                <div key={name} className="flex items-center gap-2">
                  <span className="text-xs text-needs">
                    {server.name} now also wants <code>{name}</code>
                  </span>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    aria-label={`Grant ${name} to ${server.name}`}
                    onClick={() => grant(key, name)}
                  >
                    Grant
                  </Button>
                </div>
              ),
            )}
          </div>
        );
      })}
      <div role="group" aria-label="Shell" className="flex flex-col gap-1">
        <span className="text-sm font-medium">Shell</span>
        <p className="text-xs text-muted-foreground">
          Set in the agent's own environment, where its shell commands run. The agent can read
          these values and could send them anywhere its tools reach.
        </p>
        {shell.map((name) => (
          <div key={name} className="flex items-center gap-2">
            <span className="font-mono text-xs">
              {name}
              <span className="text-muted-foreground">{missing(name)}</span>
            </span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              aria-label={`Stop granting ${name} to the shell`}
              onClick={() =>
                props.onChange({ ...props.variables, [SHELL]: shell.filter((n) => n !== name) })
              }
            >
              Remove
            </Button>
          </div>
        ))}
        <div className="flex gap-2">
          <Input
            aria-label="Variable for the shell"
            className="w-64 font-mono"
            value={shellName}
            onChange={(e) => setShellName(e.target.value)}
          />
          <Button type="button" variant="outline" size="sm" onClick={addShell}>
            Grant to the shell
          </Button>
        </div>
        {tried && !validVariableName(shellName) && (
          <p className="text-xs text-destructive">
            Use a letter or _, then letters, digits or _ (at most 128).
          </p>
        )}
      </div>
    </div>
  );
}
