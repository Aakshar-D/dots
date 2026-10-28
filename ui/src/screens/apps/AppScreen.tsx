import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";

import { commands } from "@/api/commands";
import type { AppConfig, AppRow } from "@/api/types";
import { AppFrame } from "@/components/AppFrame";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import { Button } from "@/components/ui/button";
import { appSrc } from "@/lib/app-bridge";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";
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

/// Whether to ask the person, once, to let a bundled app start dots (app library spec section 8):
/// it asks for the grant, the switch is off, and the person has not answered or set it.
export const asksToStart = (row: AppRow, config: AppConfig) =>
  config.origin.kind === "bundled" &&
  row.permissions.includes("start_dots") &&
  !config.can_start_dots &&
  !config.start_asked;

/// The one-time prompt above a bundled app's screen: "Switch on" or "Not now".
function StartPrompt({ name, title }: { name: string; title: string }) {
  const qc = useQueryClient();
  const answer = useMutation({
    mutationFn: (on: boolean) => commands.appAnswerStartPrompt(name, on),
    onSuccess: () => invalidate(qc, keys.apps),
    onError: notifyError,
  });
  return (
    <div
      role="region"
      aria-label={`Let ${title} start dots?`}
      className="flex flex-wrap items-center gap-3 border-b border-needs-border bg-needs-surface px-4 py-2 text-sm"
    >
      <p className="flex-1">
        Let {title} start dots? When you assign a task to a dot, {title} starts it on the task.
        You can change this on the Apps screen.
      </p>
      <Button size="sm" disabled={answer.isPending} onClick={() => answer.mutate(true)}>
        Switch on
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={answer.isPending}
        onClick={() => answer.mutate(false)}
      >
        Not now
      </Button>
    </div>
  );
}

/// `/apps/:name` (app library spec section 10): the app's screen filling the content area. Its
/// calls go to the app as the person, and a run it started opens in the dot's conversation.
export function AppScreen() {
  const name = useParams().name ?? "";
  const navigate = useNavigate();
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
  const openRun = async (runId: string) => {
    const link = await commands.appRunLink(name, runId);
    navigate(`/home/${encodeURIComponent(link.dot_id)}/${encodeURIComponent(link.root_run_id)}`);
  };
  return (
    <div className="flex h-full flex-col">
      {asksToStart(row, config) && <StartPrompt name={name} title={config.title} />}
      <AppFrame
        src={appSrc(name, config.version)}
        title={config.title}
        tools={row.tools.map((t) => t.name)}
        call={(tool, input) => commands.appCall(name, tool, input)}
        openRun={openRun}
        changes={changes}
      />
    </div>
  );
}
