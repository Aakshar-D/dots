import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { useSearchParams } from "react-router";

import { commands } from "@/api/commands";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { keys } from "@/lib/query";

import { ContextTab } from "./ContextTab";
import { EnvTab } from "./EnvTab";
import { McpServersTab, serverStatus } from "./McpServersTab";
import type { ContextEntry, PluginEntry, ServerEntry, SkillEntry } from "./parts";
import { PluginsTab } from "./PluginsTab";
import { SkillsTab } from "./SkillsTab";
import { ToolsReview } from "./ToolsReview";

/// The Library: the MCP servers, skills, plugins, context collections and variables that dots
/// can use. `?review=<server id>`
/// opens the review of that server's changed tool list (the Inbox links to it).
export function LibraryScreen() {
  const library = useQuery({ queryKey: keys.library, queryFn: commands.libraryList });
  const [params, setParams] = useSearchParams();
  const [tab, setTab] = useState("servers");

  if (library.isPending) return <Page title="Library" />;
  // A failed refetch keeps the list (and an open form) and shows the error above it.
  if (library.isError && !library.data) {
    return (
      <Page title="Library">
        <ErrorNote error={library.error} />
      </Page>
    );
  }
  const entries = library.data ?? [];
  const servers = entries.filter((e): e is ServerEntry => e.kind === "mcp_server");
  const skills = entries.filter((e): e is SkillEntry => e.kind === "skill");
  const plugins = entries.filter((e): e is PluginEntry => e.kind === "plugin");
  const contexts = entries.filter((e): e is ContextEntry => e.kind === "context");

  const reviewId = params.get("review");
  const reviewing = servers.find((s) => s.id === reviewId && serverStatus(s) === "changed");
  const review = (id: string | null) =>
    setParams(id ? { review: id } : {}, { replace: true });

  return (
    <Page title="Library">
      {library.isError && <ErrorNote error={library.error} />}
      <Tabs value={tab} onValueChange={setTab}>
        <TabsList>
          <TabsTrigger value="servers">MCP servers</TabsTrigger>
          <TabsTrigger value="skills">Skills</TabsTrigger>
          <TabsTrigger value="plugins">Plugins</TabsTrigger>
          <TabsTrigger value="context">Context</TabsTrigger>
          <TabsTrigger value="env">Env</TabsTrigger>
        </TabsList>
        <TabsContent value="servers">
          <McpServersTab entries={servers} plugins={plugins} onReview={review} />
        </TabsContent>
        <TabsContent value="skills">
          <SkillsTab entries={skills} />
        </TabsContent>
        <TabsContent value="plugins">
          <PluginsTab entries={plugins} />
        </TabsContent>
        <TabsContent value="context">
          <ContextTab entries={contexts} />
        </TabsContent>
        <TabsContent value="env">
          <EnvTab />
        </TabsContent>
      </Tabs>
      {reviewing && <ToolsReview entry={reviewing} onClose={() => review(null)} />}
    </Page>
  );
}
