import { useQuery } from "@tanstack/react-query";
import {
  Bot,
  Bug,
  Calendar,
  Cloud,
  Code,
  CreditCard,
  FileText,
  HardDrive,
  ListTodo,
  Mail,
  MessageSquare,
  Plug,
  Search,
  SquareKanban,
  Table,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { ConnectorEntry } from "@/api/types";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { keys } from "@/lib/query";

import { ConnectorPanel } from "./ConnectorPanel";
import { CATEGORY, makerLabel, STATE } from "./fields";

const ICONS: Record<string, LucideIcon> = {
  gmail: Mail,
  github: Code,
  slack: MessageSquare,
  "brave-search": Search,
  huggingface: Bot,
  notion: FileText,
  linear: ListTodo,
  atlassian: SquareKanban,
  sentry: Bug,
  todoist: ListTodo,
  airtable: Table,
  stripe: CreditCard,
  cloudflare: Cloud,
  "google-calendar": Calendar,
  "google-drive": HardDrive,
};

function ConnectorCard(props: { connector: ConnectorEntry; onOpen: () => void }) {
  const { connector } = props;
  const Icon = ICONS[connector.icon] ?? Plug;
  const state = STATE[connector.state];
  const action =
    connector.state === "coming_soon"
      ? "Details"
      : connector.state === "not_connected"
        ? connector.auth === "oauth"
          ? "Sign in"
          : "Connect"
        : "Manage";
  return (
    <li
      aria-label={connector.name}
      className="flex flex-col gap-2 rounded-lg border p-3"
    >
      <div className="flex items-start gap-3">
        <div className="flex size-9 shrink-0 items-center justify-center rounded-md bg-muted">
          <Icon className="size-5 text-muted-foreground" aria-hidden />
        </div>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="font-medium">{connector.name}</span>
          <span className="text-xs text-muted-foreground">{makerLabel(connector)}</span>
        </div>
        <Badge variant="secondary" className={state.className}>
          {state.label}
        </Badge>
      </div>
      <p className="text-xs text-muted-foreground">{connector.description}</p>
      {connector.used_by.length > 0 && (
        <span className="text-xs text-muted-foreground">
          {`Used by ${connector.used_by.map((d) => d.name).join(", ")}`}
        </span>
      )}
      {connector.error && connector.state !== "name_taken" && (
        <span className="text-xs text-destructive">{connector.error}</span>
      )}
      <div className="mt-auto">
        <Button
          size="sm"
          variant={connector.state === "not_connected" ? "default" : "outline"}
          aria-label={`${action} ${connector.name}`}
          onClick={props.onOpen}
        >
          {action}
        </Button>
      </div>
    </li>
  );
}

/// The Connectors screen (connectors spec §6): the apps dots can use, each connected with a key
/// or a sign-in, grouped by what they are for. A card opens the connector's panel.
export function ConnectorsScreen() {
  const list = useQuery({ queryKey: keys.connectors, queryFn: commands.connectorList });
  // Kept by id and looked up on each render, so a refetch shows in the open panel.
  const [open, setOpen] = useState<string | null>(null);
  const actions = (
    <Button size="sm" variant="outline" asChild>
      <Link to="/library">Add your own server</Link>
    </Button>
  );
  if (list.isPending) return <Page title="Connectors" actions={actions} />;
  if (list.isError && !list.data) {
    return (
      <Page title="Connectors" actions={actions}>
        <ErrorNote error={list.error} />
      </Page>
    );
  }
  const connectors = list.data ?? [];
  const opened = connectors.find((c) => c.id === open);
  const groups = Object.entries(CATEGORY)
    .map(([category, title]) => ({
      title,
      items: connectors.filter((c) => c.category === category),
    }))
    .filter((g) => g.items.length > 0);
  return (
    <Page title="Connectors" actions={actions}>
      {list.isError && <ErrorNote error={list.error} />}
      <p className="text-xs text-muted-foreground">
        Connect the apps you use so your sols can work with them. Each one shows how to get its
        key or sign in. Keys and sign-ins are kept in Windows Credential Manager, and only the
        sols you pick can use them. For any other MCP server, add it in the Library.
      </p>
      {groups.map((group) => (
        <section key={group.title} aria-label={group.title} className="flex flex-col gap-2">
          <h2 className="text-sm font-semibold">{group.title}</h2>
          <ul className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
            {group.items.map((c) => (
              <ConnectorCard key={c.id} connector={c} onOpen={() => setOpen(c.id)} />
            ))}
          </ul>
        </section>
      ))}
      {opened && (
        <ConnectorPanel key={opened.id} connector={opened} onClose={() => setOpen(null)} />
      )}
    </Page>
  );
}
