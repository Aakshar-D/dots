import { useQuery } from "@tanstack/react-query";
import { Fragment, useId, useState } from "react";

import { commands } from "@/api/commands";
import type { EntryKind, GrantStatus, LibraryEntry, OrgSummary, Uses } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { mayUse, useOrgList } from "@/lib/dot-org";
import { keys } from "@/lib/query";
import { validVariableName } from "@/screens/library/EnvTab";
import { variablesIn } from "@/screens/library/text";

type Grants = Record<string, string[]>;
type Server = Extract<LibraryEntry, { kind: "mcp_server" }>;
type Entry = { name: string; kind: EntryKind };

const SHELL = "shell";
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const has = (names: string[], name: string) => names.some((n) => same(n, name));

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

/// The names a dot of the org `orgId` reads, each with the kind of the value that would
/// resolve: the org's entries, and Personal's where the org's `personal` mode lets its dots
/// read them, the side the mode puts first winning a name both have (env vault spec §8). No
/// accepted policy reads as `deny`, and an org not joined (`org` null) gives nothing. A
/// Personal dot (`orgId` null) reads Personal's.
function scopeEntries(
  orgId: string | null,
  org: OrgSummary | null,
  personal: Entry[],
  inOrg: Entry[],
): Entry[] {
  if (orgId === null) return personal;
  if (org === null) return [];
  const lacking = (from: Entry[], have: Entry[]) =>
    from.filter((e) => !have.some((h) => same(h.name, e.name)));
  switch (org.personal_mode ?? "deny") {
    case "deny":
      return inOrg;
    case "fallback":
      return [...inOrg, ...lacking(personal, inOrg)];
    case "override":
      return [...personal, ...lacking(inOrg, personal)];
  }
}

/// A granted name with its status in the dot's scope. A Personal dot shows, as before, only
/// that Personal lacks it. An org dot's resolves "from" the org or Personal, or gives the
/// core's reason: in the attention tone, or the error tone when the org was removed. A grant
/// the stored dot does not have yet has no status until the dot is saved.
function GrantLine(props: {
  name: string;
  status: GrantStatus | undefined;
  orgName: string | null;
  lacksInPersonal: boolean;
}) {
  const { name, status, orgName } = props;
  const plain = (note: string) => (
    <span className="font-mono text-xs">
      {name}
      <span className="text-muted-foreground">{note}</span>
    </span>
  );
  if (orgName === null) {
    const lacks = status ? status.status !== "ok" : props.lacksInPersonal;
    return plain(lacks ? " · not in Personal" : "");
  }
  if (!status) return plain(" · checked when you save");
  if (status.status === "ok") {
    return plain(` · from ${status.source === "personal" ? "Personal" : orgName}`);
  }
  const tone = status.status === "org_removed" ? "text-destructive" : "text-needs";
  return (
    <span className="flex flex-col">
      <span className="font-mono text-xs">{name}</span>
      <span className={`text-xs ${tone}`}>{status.text}</span>
    </span>
  );
}

/// A dot's grants (env vault spec §7): what each server it uses may read, and what its shell
/// gets. A server the dot did not use before (`before` is the stored dot's `uses`) is granted
/// its variables when the dot is saved; a variable a server asks for later waits for Grant.
/// Each grant of a stored dot (`dotId`) shows its status in the dot's scope (`orgId`, null for
/// Personal, §9). The shell is offered the scope's names, without the names an org's `use`
/// rules deny and the secrets an org keeps from the shell.
export function Variables(props: {
  uses: Uses;
  variables: Grants;
  before: Uses | null;
  orgId: string | null;
  dotId: string | null;
  onChange: (variables: Grants) => void;
}) {
  const { orgId, dotId } = props;
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const env = useQuery({ queryKey: keys.env, queryFn: commands.envList });
  const orgs = useOrgList(orgId !== null);
  const org = orgs?.find((o) => o.id === orgId) ?? null;
  const orgEntries = useQuery({
    queryKey: keys.orgEntries(orgId ?? ""),
    queryFn: () => commands.orgEntries(orgId ?? ""),
    enabled: org !== null,
  });
  // What resolves depends on Personal, the org and the Library, so it is asked for each time
  // the section is shown; an org event marks it stale too.
  const statuses = useQuery({
    queryKey: keys.dotVariables(dotId ?? ""),
    queryFn: () => commands.dotVariables(dotId ?? ""),
    enabled: dotId !== null,
    staleTime: 0,
  });
  const listId = useId();
  const [shellName, setShellName] = useState("");
  const [tried, setTried] = useState(false);
  if (library.isError) return <ErrorNote error={library.error} />;
  const entries = library.data ?? [];
  const personal = env.data ?? [];
  const had = usedServers(entries, props.before).map((s) => s.name);
  const shell = props.variables[SHELL] ?? [];
  const orgName = orgId === null ? null : (org?.name ?? orgId);
  const grant = (key: string, name: string) =>
    props.onChange({ ...props.variables, [key]: [...(props.variables[key] ?? []), name] });
  const line = (key: string, name: string) => (
    <GrantLine
      name={name}
      status={statuses.data?.find((s) => s.consumer === key && same(s.name, name))}
      orgName={orgName}
      lacksInPersonal={env.data !== undefined && !has(personal.map((e) => e.name), name)}
    />
  );

  // What the shell may be offered: the scope's names, less an org's secrets when the org keeps
  // secrets from the shell (a Personal secret too, for a dot of that org), and less the names
  // the org's `use` rules deny, whichever side the value comes from.
  const named = scopeEntries(orgId, org, personal, orgEntries.data ?? []);
  const keptBack = org !== null && !org.shell_secrets;
  const hidden = keptBack ? named.filter((e) => e.kind === "secret").map((e) => e.name) : [];
  const offered = named
    .filter((e) => !has(hidden, e.name) && !has(shell, e.name))
    .filter((e) => org === null || mayUse(org.use_rules, e.name))
    .map((e) => e.name);
  const refused = has(hidden, shellName);

  function addShell() {
    setTried(true);
    if (!validVariableName(shellName) || has(shell, shellName) || refused) return;
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
                <Fragment key={name}>{line(key, name)}</Fragment>
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
            {line(SHELL, name)}
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
            list={listId}
            value={shellName}
            onChange={(e) => setShellName(e.target.value)}
          />
          <datalist id={listId}>
            {offered.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
          <Button type="button" variant="outline" size="sm" onClick={addShell}>
            Grant to the shell
          </Button>
        </div>
        {keptBack && (
          <p className="text-xs text-muted-foreground">
            {orgName} does not allow secrets in the shell, so its secrets are not offered here.
          </p>
        )}
        {tried && !validVariableName(shellName) && (
          <p className="text-xs text-destructive">
            Use a letter or _, then letters, digits or _ (at most 128).
          </p>
        )}
        {tried && validVariableName(shellName) && refused && (
          <p className="text-xs text-destructive">
            {shellName} is a secret, and {orgName} does not allow secrets in the shell.
          </p>
        )}
      </div>
    </div>
  );
}
