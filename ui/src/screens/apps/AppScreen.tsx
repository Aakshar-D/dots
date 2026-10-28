import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";

import { commands } from "@/api/commands";
import { AppFrame } from "@/components/AppFrame";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { appSrc } from "@/lib/app-bridge";
import { keys } from "@/lib/query";
import { appDataChanges } from "@/lib/runtime-events";
import { appConfig } from "@/screens/apps/AppsScreen";

/// How many times the app `name`'s data changed since the screen opened.
function useDataChanges(name: string): number {
  const [changes, setChanges] = useState(0);
  useEffect(() => {
    const heard = () => setChanges((n) => n + 1);
    appDataChanges.addEventListener(name, heard);
    return () => appDataChanges.removeEventListener(name, heard);
  }, [name]);
  return changes;
}

/// `/apps/:name` (app library spec section 10): the app's screen filling the content area. Its
/// calls go to the app as the person.
export function AppScreen() {
  const name = useParams().name ?? "";
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
  const changes = useDataChanges(name);
  const row = apps.data?.find((r) => r.item.name === name);
  const config = row ? appConfig(row) : null;
  if (apps.isPending) return <Page title="Apps" />;
  if (apps.isError || !row || !config || row.error) {
    return (
      <Page title={config?.title ?? name}>
        {apps.isError ? (
          <ErrorNote error={apps.error} />
        ) : row?.error ? (
          <ErrorNote error={`The app ${config?.title ?? name} cannot open: ${row.error}`} />
        ) : (
          <p className="text-muted-foreground">There is no app {name}.</p>
        )}
        <Link to="/apps" className="text-sm hover:underline">
          All apps
        </Link>
      </Page>
    );
  }
  return (
    <div className="flex h-full flex-col">
      <AppFrame
        src={appSrc(name, config.version)}
        title={config.title}
        tools={row.tools.map((t) => t.name)}
        call={(tool, input) => commands.appCall(name, tool, input)}
        changes={changes}
      />
    </div>
  );
}
