import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { LayoutGrid } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router";

import { commands } from "@/api/commands";
import type { AppConfig, AppRow, KeptData } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNote } from "@/components/ErrorNote";
import { Page } from "@/components/Page";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { formatBytes, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, invalidateDots, keys } from "@/lib/query";
import { ItemRow } from "@/screens/library/parts";

/// The app's config, from its Library item.
export function appConfig(row: AppRow): AppConfig | null {
  return row.item.kind === "app" ? row.item.config : null;
}

/// The Apps screen (app library spec section 10): the installed apps, each opening its screen,
/// with its switches, roll back and remove; then the data removed apps kept.
export function AppsScreen() {
  const apps = useQuery({ queryKey: keys.apps, queryFn: commands.appsList });
  const kept = useQuery({ queryKey: keys.appsKept, queryFn: commands.appsKeptList });
  if (apps.isPending) return <Page title="Apps" />;
  if (apps.isError) {
    return (
      <Page title="Apps">
        <ErrorNote error={apps.error} />
      </Page>
    );
  }
  const keptData = kept.data ?? [];
  return (
    <Page title="Apps">
      {apps.data.length === 0 ? (
        <EmptyState icon={LayoutGrid}>
          No apps yet. Ask a dot in chat for one ("an app to track my expenses"); it sends the app
          for your review, and the apps you add show up here.
        </EmptyState>
      ) : (
        <ul className="flex flex-col gap-2">
          {apps.data.map((row) => (
            <AppItem key={row.item.id} row={row} />
          ))}
        </ul>
      )}
      {kept.isError && <ErrorNote error={kept.error} />}
      {keptData.length > 0 && (
        <section aria-labelledby="apps-kept" className="flex flex-col gap-2">
          <h2 id="apps-kept" className="font-semibold">
            Kept data
          </h2>
          <p className="text-xs text-muted-foreground">
            Data of removed apps. A new app of the same name whose first migrations match can
            start on it when you add it.
          </p>
          <ul className="flex flex-col gap-2">
            {keptData.map((data) => (
              <KeptItem key={data.id} data={data} />
            ))}
          </ul>
        </section>
      )}
    </Page>
  );
}

function AppItem({ row }: { row: AppRow }) {
  const qc = useQueryClient();
  const config = appConfig(row);
  const name = row.item.name;
  const changed = () => {
    invalidate(qc, keys.apps);
    invalidate(qc, keys.library);
    invalidateDots(qc);
  };
  const everyDot = useMutation({
    mutationFn: (on: boolean) => commands.appSetEveryDot(name, on),
    onSuccess: changed,
    onError: notifyError,
  });
  const canStart = useMutation({
    mutationFn: (on: boolean) => commands.appSetCanStartDots(name, on),
    onSuccess: changed,
    onError: notifyError,
  });
  const rollBack = useMutation({
    mutationFn: () => commands.appRollBack(name),
    onSuccess: changed,
    onError: notifyError,
  });
  if (config === null) return null;
  const title = config.title;
  const asksToStart = row.permissions.includes("start_dots");
  return (
    <ItemRow
      label={title}
      actions={
        <>
          {config.previous_version !== null && (
            <ConfirmButton
              title={`Roll back ${title}?`}
              description={`Version ${config.previous_version} becomes the active one again. Its data stays as it is: the update after it changed no data schema.`}
              confirmLabel="Roll back"
              ariaLabel={`Roll back ${title}`}
              disabled={rollBack.isPending}
              onConfirm={() => rollBack.mutate()}
            >
              Roll back
            </ConfirmButton>
          )}
          <RemoveApp row={row} title={title} />
        </>
      }
    >
      <div className="flex flex-wrap items-center gap-2">
        <Link to={`/apps/${encodeURIComponent(name)}`} className="font-medium hover:underline">
          {title}
        </Link>
        <span className="text-xs text-muted-foreground">version {config.version}</span>
        {config.every_dot && <Badge variant="secondary">Every dot</Badge>}
        {config.origin.kind === "bundled" && !config.changed && (
          <Badge variant="outline">Comes with Dots</Badge>
        )}
      </div>
      <span className="text-xs text-muted-foreground">{config.description}</span>
      {config.changed && (
        <span className="text-xs text-muted-foreground">
          Changed by you; Dots no longer updates it.
        </span>
      )}
      {row.error && <span className="text-xs text-destructive">{row.error}</span>}
      <span className="text-xs text-muted-foreground">
        {row.dots.length > 0 ? `Used by ${row.dots.join(", ")}` : "No dot uses it yet"}
      </span>
      <div className="flex flex-wrap items-center gap-4 pt-1 text-xs">
        <label className="flex items-center gap-2">
          <Switch
            checked={config.every_dot}
            disabled={everyDot.isPending}
            onCheckedChange={(on) => everyDot.mutate(on)}
          />
          Every dot
        </label>
        {asksToStart && (
          <label className="flex items-center gap-2">
            <Switch
              checked={config.can_start_dots}
              disabled={canStart.isPending}
              onCheckedChange={(on) => canStart.mutate(on)}
            />
            Can start dots
          </label>
        )}
      </div>
    </ItemRow>
  );
}

/// "Remove" (app library spec section 10): the first dialog is the person's confirmation, also
/// of the dots that use the app; then a second one for the data, which stays unless the person
/// says to delete it too.
function RemoveApp({ row, title }: { row: AppRow; title: string }) {
  const qc = useQueryClient();
  const [step, setStep] = useState<"closed" | "app" | "data">("closed");
  const remove = useMutation({
    mutationFn: (deleteData: boolean) =>
      commands.appRemove(row.item.name, true, deleteData),
    onSuccess: () => {
      invalidate(qc, keys.apps);
      invalidate(qc, keys.library);
      invalidateDots(qc);
    },
    onError: notifyError,
    onSettled: () => setStep("closed"),
  });
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        aria-label={`Remove ${title}`}
        onClick={() => setStep("app")}
      >
        Remove
      </Button>
      <AlertDialog open={step === "app"} onOpenChange={(open) => !open && setStep("closed")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove {title}?</AlertDialogTitle>
            <AlertDialogDescription>
              {row.dots.length > 0
                ? `Used by ${row.dots.join(", ")}. Removing it takes it out of them.`
                : "No dot uses it."}{" "}
              Its code is deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button onClick={() => setStep("data")}>Remove app</Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={step === "data"} onOpenChange={(open) => !open && setStep("closed")}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete {title}'s data too?</AlertDialogTitle>
            <AlertDialogDescription>
              Kept data stays on this PC under Kept data, and a new {title} can start on it.
              Deleted data cannot be brought back.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <Button
              variant="outline"
              disabled={remove.isPending}
              onClick={() => remove.mutate(false)}
            >
              Keep the data
            </Button>
            <AlertDialogAction
              disabled={remove.isPending}
              onClick={(e) => {
                e.preventDefault();
                remove.mutate(true);
              }}
            >
              Delete the data
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function KeptItem({ data }: { data: KeptData }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.appKeptDelete(data.id),
    onSuccess: () => invalidate(qc, keys.appsKept),
    onError: notifyError,
  });
  return (
    <ItemRow
      label={`${data.title}, kept data`}
      actions={
        <ConfirmButton
          title={`Delete the data of ${data.title}?`}
          description="It cannot be brought back, and no new app can start on it."
          confirmLabel="Delete data"
          disabled={remove.isPending}
          onConfirm={() => remove.mutate()}
        >
          Delete data
        </ConfirmButton>
      }
    >
      <span className="font-medium">{data.title}</span>
      <span className="text-xs text-muted-foreground">
        Removed {relativeTime(data.removed_at)} · {formatBytes(data.bytes)}
      </span>
    </ItemRow>
  );
}
