import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { EngineKind, LibraryEntry, LibraryKind, Uses } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { keys } from "@/lib/query";
import { plural } from "@/screens/library/parts";

const GROUPS: { kind: LibraryKind; key: keyof Uses; title: string }[] = [
  { kind: "mcp_server", key: "mcp_servers", title: "MCP servers" },
  { kind: "skill", key: "skills", title: "Skills" },
  { kind: "plugin", key: "plugins", title: "Plugins" },
  { kind: "context", key: "context", title: "Context" },
];

/// A context collection every dot reads, ticked or not.
const everyDot = (entry: LibraryEntry) => entry.kind === "context" && entry.config.every_dot;

/// The parts of a plugin that a local dot does not use, as a phrase ("1 command, 2 agents and
/// its hooks"), or null when it has none.
function unusedParts(entry: LibraryEntry): string | null {
  const plugin = entry.plugin;
  if (!plugin) return null;
  const parts = [
    plugin.commands.length > 0 && plural(plugin.commands.length, "command"),
    plugin.agents.length > 0 && plural(plugin.agents.length, "agent"),
    plugin.has_hooks && "its hooks",
  ].filter((part): part is string => part !== false);
  if (parts.length < 2) return parts[0] ?? null;
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/// What a row says besides the name.
function Detail({ entry }: { entry: LibraryEntry }) {
  const text =
    entry.kind === "skill"
      ? entry.skill?.description
      : entry.kind === "plugin"
        ? entry.plugin?.description
        : entry.kind === "context"
          ? [
              entry.config.description,
              entry.context &&
                `${entry.context.complete ? "" : "at least "}${plural(entry.context.documents, "document")}`,
              entry.config.every_dot && "every dot",
            ]
              .filter(Boolean)
              .join(" · ")
          : null;
  const status = entry.catalog?.status;
  return (
    <>
      {text && <span className="text-xs text-muted-foreground">{text}</span>}
      {entry.problem && <span className="text-xs text-destructive">{entry.problem}</span>}
      {status === "changed" && (
        <span className="text-xs text-needs">
          Tools changed: they ask first until you review them.
        </span>
      )}
      {status === "unreachable" && (
        <span className="text-xs text-needs">
          Unreachable at the last check.
        </span>
      )}
    </>
  );
}

/// The dot editor's "Tools and skills" section: which Library items the dot uses, as four
/// checklists. A plugin's own MCP servers are not listed: they come with the plugin. A context
/// collection for every dot shows ticked and cannot be unticked here.
export function ToolsAndSkills(props: {
  uses: Uses;
  engine: EngineKind;
  onChange: (uses: Uses) => void;
}) {
  const { uses } = props;
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  if (library.isPending) return <p className="text-muted-foreground">Loading the Library…</p>;
  if (library.isError) return <ErrorNote error={library.error} />;
  const entries = library.data;

  function toggle(key: keyof Uses, name: string, on: boolean) {
    const rest = uses[key].filter((n) => n !== name);
    props.onChange({ ...uses, [key]: on ? [...rest, name] : rest });
  }

  // The variables the dot's servers need, its plugins' servers included.
  const usedPlugins = entries.filter((e) => e.kind === "plugin" && uses.plugins.includes(e.name));
  const missing = [
    ...new Set(
      entries
        .filter(
          (e) =>
            e.kind === "mcp_server" &&
            (e.owner_id === null
              ? uses.mcp_servers.includes(e.name)
              : usedPlugins.some((p) => p.id === e.owner_id)),
        )
        .flatMap((e) => e.missing_variables),
    ),
  ];
  const notes =
    props.engine === "local"
      ? usedPlugins.flatMap((p) => {
          const parts = unusedParts(p);
          return parts
            ? [
                `On a local dot, ${p.name} gives its skills and MCP servers only. Its ${parts} do not apply.`,
              ]
            : [];
        })
      : [];

  return (
    <div className="flex flex-col gap-4">
      {GROUPS.map(({ kind, key, title }) => {
        const items = entries.filter((e) => e.kind === kind && e.owner_id === null);
        const known = new Set(items.map((e) => e.name));
        const stale = uses[key].filter((name) => !known.has(name));
        return (
          <fieldset key={kind} aria-label={title} className="flex flex-col gap-1.5">
            <legend className="mb-1 text-xs font-medium text-muted-foreground">{title}</legend>
            {items.length === 0 && stale.length === 0 && (
              <p className="text-xs text-muted-foreground">
                Nothing here yet. Add some in the <Link to="/library" className="underline">Library</Link>.
              </p>
            )}
            <ul className="flex flex-col gap-1.5">
              {items.map((entry) => (
                <li key={entry.id} className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={entry.name}
                    checked={everyDot(entry) || uses[key].includes(entry.name)}
                    disabled={everyDot(entry)}
                    onChange={(e) => toggle(key, entry.name, e.target.checked)}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium" aria-hidden>
                      {entry.name}
                    </span>
                    <Detail entry={entry} />
                  </div>
                </li>
              ))}
              {stale.map((name) => (
                <li key={name} className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    className="mt-1"
                    aria-label={name}
                    checked
                    onChange={() => toggle(key, name, false)}
                  />
                  <div className="flex min-w-0 flex-col">
                    <span className="font-medium" aria-hidden>
                      {name}
                    </span>
                    <span className="text-xs text-destructive">Not in the Library any more.</span>
                  </div>
                </li>
              ))}
            </ul>
          </fieldset>
        );
      })}
      {notes.map((note) => (
        <p key={note} className="text-xs text-muted-foreground">
          {note}
        </p>
      ))}
      {missing.length > 0 && (
        <p role="status" className="text-xs text-needs">
          Not set in Dots' environment: {missing.join(", ")}. A server that needs one cannot start
          until it is set.
        </p>
      )}
    </div>
  );
}
